use super::*;
use gestura_core_agents::AgentExecutionMode;
use gestura_core_tools::git::GitTools;
use std::fs;
use std::path::{Path, PathBuf};

impl<M: OrchestratorAgentManager> AgentOrchestrator<M> {
    pub(crate) async fn prepare_environment(
        &self,
        task: &DelegatedTask,
    ) -> Result<EnvironmentRecord, String> {
        let spec = self.build_environment_spec(task)?;
        let now = Utc::now();
        let mut record = EnvironmentRecord {
            id: spec.id.clone(),
            prepared_path: spec.prepared_path.clone(),
            spec,
            state: EnvironmentState::Provisioning,
            health: EnvironmentHealth::Unknown,
            lease: None,
            cleanup_result: None,
            recovery_status: RecoveryStatus::NotRequired,
            recovery_action: None,
            failure: None,
            created_at: now,
            updated_at: now,
            last_verified_at: None,
            metadata: None,
        };

        let preparation_result = match record.spec.execution_mode {
            AgentExecutionMode::SharedWorkspace | AgentExecutionMode::Remote => {
                self.prepare_shared_environment(&mut record)
            }
            AgentExecutionMode::IsolatedWorkspace => self.prepare_isolated_environment(&mut record),
            AgentExecutionMode::GitWorktree => self.prepare_git_worktree_environment(&mut record),
        };

        if let Err((kind, message)) = preparation_result {
            record.state = EnvironmentState::Failed;
            record.health = EnvironmentHealth::Unknown;
            record.recovery_status = RecoveryStatus::Pending;
            record.recovery_action = Some(RecoveryAction::MarkTaskBlocked);
            record.failure = Some(EnvironmentFailure {
                kind,
                message,
                command: None,
                stderr: None,
                occurred_at: Utc::now(),
            });
        }

        record.updated_at = Utc::now();
        record.last_verified_at = Some(Utc::now());
        self.persist_environment_record(&record).await?;
        Ok(record)
    }

    pub(crate) async fn acquire_environment_lease(
        &self,
        environment_id: &str,
        task_id: &str,
        agent_id: &str,
    ) -> Result<EnvironmentRecord, String> {
        let mut environments = self.environments.lock().await;
        let record = environments
            .get_mut(environment_id)
            .ok_or_else(|| format!("Environment {environment_id} not found"))?;

        if let Some(lease) = &record.lease
            && lease.released_at.is_none()
            && lease.task_id != task_id
        {
            return Err(format!(
                "Environment {environment_id} is already leased to task {}",
                lease.task_id
            ));
        }

        record.lease = Some(EnvironmentLease {
            task_id: task_id.to_string(),
            agent_id: agent_id.to_string(),
            lease_kind: EnvironmentLeaseKind::Execution,
            acquired_at: Utc::now(),
            released_at: None,
        });
        record.state = EnvironmentState::InUse;
        record.updated_at = Utc::now();
        let snapshot = record.clone();
        drop(environments);
        self.persist_environment_record(&snapshot).await?;
        Ok(snapshot)
    }

    pub(crate) async fn release_environment_lease(
        &self,
        environment_id: &str,
    ) -> Result<Option<EnvironmentRecord>, String> {
        let mut environments = self.environments.lock().await;
        let Some(record) = environments.get_mut(environment_id) else {
            return Ok(None);
        };

        if let Some(lease) = record.lease.as_mut() {
            lease.released_at = Some(Utc::now());
        }
        if !matches!(
            record.state,
            EnvironmentState::Archived | EnvironmentState::Removed | EnvironmentState::Failed
        ) {
            record.state = EnvironmentState::Ready;
        }
        record.updated_at = Utc::now();
        let snapshot = record.clone();
        drop(environments);
        self.persist_environment_record(&snapshot).await?;
        Ok(Some(snapshot))
    }

    pub(crate) async fn finalize_environment_for_task(
        &self,
        environment_id: &str,
        success: bool,
        force_archive_dirty: bool,
    ) -> Result<Option<EnvironmentRecord>, String> {
        let mut environments = self.environments.lock().await;
        let Some(record) = environments.get_mut(environment_id) else {
            return Ok(None);
        };

        if let Some(lease) = record.lease.as_mut() {
            lease.released_at = Some(Utc::now());
        }

        let cleanup_result = self.apply_cleanup_policy(record, success, force_archive_dirty)?;
        record.cleanup_result = Some(cleanup_result.clone());
        record.updated_at = Utc::now();
        let snapshot = record.clone();
        drop(environments);
        self.persist_environment_record(&snapshot).await?;
        if let Some(observer) = self.observer.read().await.clone() {
            let environment_id = snapshot.id.clone();
            tokio::spawn(async move {
                observer
                    .on_environment_cleanup(environment_id, cleanup_result)
                    .await;
            });
        }
        Ok(Some(snapshot))
    }

    pub(crate) async fn persist_environment_record(
        &self,
        record: &EnvironmentRecord,
    ) -> Result<(), String> {
        let root = record.spec.workspace_root.clone();
        persist_environment_to_disk(&root, record)?;
        self.environments
            .lock()
            .await
            .insert(record.id.clone(), record.clone());
        if let Some(observer) = self.observer.read().await.clone() {
            let snapshot = record.clone();
            tokio::spawn(async move {
                observer.on_environment_updated(snapshot).await;
            });
        }
        Ok(())
    }

    pub async fn list_environments(&self, run_id: Option<&str>) -> Vec<EnvironmentRecord> {
        let environments = self.environments.lock().await;
        let mut records: Vec<_> = environments
            .values()
            .filter(|record| {
                run_id
                    .map(|value| record.spec.run_id == value)
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        records.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        records
    }

    pub async fn get_environment(&self, environment_id: &str) -> Option<EnvironmentRecord> {
        if let Some(environment) = self.environments.lock().await.get(environment_id).cloned() {
            return Some(environment);
        }

        self.default_workspace_dir
            .as_ref()
            .and_then(|root| load_persisted_environment_by_id(root, environment_id))
    }

    pub async fn retry_environment_preparation(
        &self,
        environment_id: &str,
    ) -> Result<EnvironmentRecord, String> {
        let existing = self
            .get_environment(environment_id)
            .await
            .ok_or_else(|| format!("Environment {environment_id} not found"))?;

        let task = self
            .find_task_by_environment_id(environment_id)
            .await
            .ok_or_else(|| format!("No task found for environment {environment_id}"))?;

        let mut refreshed = self.prepare_environment(&task).await?;
        refreshed.spec.cleanup_policy = existing.spec.cleanup_policy;
        self.update_environment_in_runs(&refreshed).await?;
        Ok(refreshed)
    }

    pub async fn cleanup_environment(
        &self,
        environment_id: &str,
        archive_if_dirty: bool,
    ) -> Result<EnvironmentRecord, String> {
        let updated = self
            .finalize_environment_for_task(environment_id, true, archive_if_dirty)
            .await?
            .ok_or_else(|| format!("Environment {environment_id} not found"))?;
        self.update_environment_in_runs(&updated).await?;
        Ok(updated)
    }

    pub(crate) async fn update_environment_in_runs(
        &self,
        environment: &EnvironmentRecord,
    ) -> Result<(), String> {
        let mut runs = self.supervisor_runs.lock().await;
        let mut affected_runs = Vec::new();
        for run in runs.values_mut() {
            let mut changed = false;
            for task_record in &mut run.tasks {
                if task_record.environment_id == environment.id {
                    task_record.environment = environment.summary();
                    task_record.updated_at = Utc::now();
                    changed = true;
                }
            }
            if changed {
                run.updated_at = Utc::now();
                affected_runs.push(run.clone());
            }
        }
        drop(runs);

        for run in affected_runs {
            self.persist_run(&run)?;
            self.notify_run_updated(run).await;
        }

        Ok(())
    }

    pub(crate) async fn find_task_by_environment_id(
        &self,
        environment_id: &str,
    ) -> Option<DelegatedTask> {
        let runs = self.supervisor_runs.lock().await;
        runs.values()
            .flat_map(|run| run.tasks.iter())
            .find(|record| record.environment_id == environment_id)
            .map(|record| record.task.clone())
    }

    fn build_environment_spec(&self, task: &DelegatedTask) -> Result<EnvironmentSpec, String> {
        let workspace_root = task
            .workspace_dir
            .clone()
            .or_else(|| self.default_workspace_dir.clone())
            .ok_or_else(|| format!("Task {} is missing a workspace root", task.id))?;

        let session_workspace =
            self.session_workspace(&workspace_root, task.session_id.as_deref())?;
        let run_id = task
            .run_id
            .clone()
            .ok_or_else(|| format!("Task {} is missing run_id", task.id))?;
        let agent_component = sanitize_component(&task.agent_id);
        let task_component = sanitize_component(&task.id);
        let run_component = sanitize_component(&run_id);
        let session_component = sanitize_component(task.session_id.as_deref().unwrap_or("global"));
        let environment_id = format!("env_{run_component}_{task_component}_{agent_component}");

        let cleanup_policy = default_cleanup_policy(&task.execution_mode);
        let remote_url = task.remote_target.as_ref().map(|target| target.url.clone());

        let git_worktree = if matches!(task.execution_mode, AgentExecutionMode::GitWorktree) {
            let git = GitTools::new(Some(workspace_root.clone()));
            let repo_root = git
                .rev_parse_toplevel()
                .map_err(|error| format!("Failed to locate git repository root: {error}"))?;
            let base_branch = git
                .current_branch()
                .map_err(|error| format!("Failed to determine current branch: {error}"))?;
            let worktree_path = session_workspace
                .resolve_path_for_create(
                    &Path::new(".gestura")
                        .join("worktrees")
                        .join(session_component.clone())
                        .join(run_component.clone())
                        .join(agent_component.clone())
                        .join(task_component.clone()),
                )
                .map_err(|error| format!("Failed to resolve worktree path: {error}"))?;
            Some(GitWorktreeSpec {
                repo_root,
                base_branch,
                worktree_branch: format!(
                    "gestura/{}/{}/{}/{}",
                    session_component, run_component, agent_component, task_component
                ),
                worktree_path,
                create_branch_if_missing: true,
            })
        } else {
            None
        };

        let prepared_root = match task.execution_mode {
            AgentExecutionMode::SharedWorkspace | AgentExecutionMode::Remote => {
                workspace_root.clone()
            }
            AgentExecutionMode::IsolatedWorkspace => session_workspace
                .resolve_path_for_create(
                    &Path::new(".gestura")
                        .join("environments")
                        .join(session_component)
                        .join(run_component)
                        .join(agent_component)
                        .join(task_component),
                )
                .map_err(|error| format!("Failed to resolve isolated workspace path: {error}"))?,
            AgentExecutionMode::GitWorktree => git_worktree
                .as_ref()
                .map(|worktree| worktree.worktree_path.clone())
                .unwrap_or_else(|| workspace_root.clone()),
        };

        Ok(EnvironmentSpec {
            id: environment_id,
            execution_mode: task.execution_mode.clone(),
            workspace_root,
            prepared_path: prepared_root,
            session_id: task.session_id.clone(),
            run_id,
            task_id: task.id.clone(),
            agent_id: task.agent_id.clone(),
            cleanup_policy,
            write_access: !task.planning_only,
            git_worktree,
            remote_url,
        })
    }

    fn prepare_shared_environment(
        &self,
        record: &mut EnvironmentRecord,
    ) -> Result<(), (EnvironmentFailureKind, String)> {
        if !record.prepared_path.exists() {
            return Err((
                EnvironmentFailureKind::WorkspaceNotFound,
                format!(
                    "Workspace root {} does not exist",
                    record.prepared_path.display()
                ),
            ));
        }

        record.health = EnvironmentHealth::Clean;
        record.state = EnvironmentState::Ready;
        record.recovery_status = RecoveryStatus::NotRequired;
        record.recovery_action = None;
        Ok(())
    }

    fn prepare_isolated_environment(
        &self,
        record: &mut EnvironmentRecord,
    ) -> Result<(), (EnvironmentFailureKind, String)> {
        if let Some(parent) = record.prepared_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                (
                    EnvironmentFailureKind::WorkspaceNotFound,
                    format!(
                        "Failed to create parent directory {}: {error}",
                        parent.display()
                    ),
                )
            })?;
        }

        fs::create_dir_all(&record.prepared_path).map_err(|error| {
            (
                EnvironmentFailureKind::WorkspaceNotFound,
                format!(
                    "Failed to create isolated environment {}: {error}",
                    record.prepared_path.display()
                ),
            )
        })?;

        record.health = EnvironmentHealth::Clean;
        record.state = EnvironmentState::Ready;
        record.recovery_status = RecoveryStatus::NotRequired;
        record.recovery_action = None;
        Ok(())
    }

    fn prepare_git_worktree_environment(
        &self,
        record: &mut EnvironmentRecord,
    ) -> Result<(), (EnvironmentFailureKind, String)> {
        let Some(spec) = record.spec.git_worktree.as_ref() else {
            return Err((
                EnvironmentFailureKind::WorktreeInvalid,
                "Missing git worktree spec for git-worktree execution mode".to_string(),
            ));
        };
        let git = GitTools::new(Some(spec.repo_root.clone()));
        if !git.path_is_git_repo().unwrap_or(false) {
            return Err((
                EnvironmentFailureKind::NotGitRepository,
                format!("{} is not a git repository", spec.repo_root.display()),
            ));
        }

        let existing_worktree = git
            .worktree_list()
            .map_err(|error| {
                (
                    EnvironmentFailureKind::GitCommandFailed,
                    format!("Failed to list git worktrees: {error}"),
                )
            })?
            .into_iter()
            .find(|worktree| worktree.path == spec.worktree_path);

        if spec.worktree_path.exists() && existing_worktree.is_none() {
            return Err((
                EnvironmentFailureKind::WorktreeAlreadyExists,
                format!(
                    "Path {} exists but is not a registered git worktree",
                    spec.worktree_path.display()
                ),
            ));
        }

        if existing_worktree.is_none() {
            git.worktree_add(
                &spec.worktree_path,
                &spec.worktree_branch,
                &spec.base_branch,
                spec.create_branch_if_missing,
            )
            .map_err(|error| {
                (
                    EnvironmentFailureKind::WorktreeCreationFailed,
                    format!("Failed to create git worktree: {error}"),
                )
            })?;
        }

        if !spec.worktree_path.exists() {
            return Err((
                EnvironmentFailureKind::WorktreeInvalid,
                format!(
                    "Git worktree {} was not created",
                    spec.worktree_path.display()
                ),
            ));
        }

        let is_clean = git
            .is_worktree_clean(&spec.worktree_path)
            .map_err(|error| {
                (
                    EnvironmentFailureKind::GitCommandFailed,
                    format!("Failed to inspect worktree cleanliness: {error}"),
                )
            })?;

        record.health = if is_clean {
            EnvironmentHealth::Clean
        } else {
            EnvironmentHealth::Dirty
        };
        record.state = EnvironmentState::Ready;
        record.recovery_status = RecoveryStatus::NotRequired;
        record.recovery_action = None;
        Ok(())
    }

    fn apply_cleanup_policy(
        &self,
        record: &mut EnvironmentRecord,
        success: bool,
        force_archive_dirty: bool,
    ) -> Result<CleanupResult, String> {
        record.state = EnvironmentState::Cleaning;
        let disposition = match record.spec.cleanup_policy {
            CleanupPolicy::KeepAlways => CleanupDisposition::Kept,
            CleanupPolicy::RemoveOnSuccess if success => CleanupDisposition::Removed,
            CleanupPolicy::ArchiveOnFailure if !success => CleanupDisposition::Archived,
            CleanupPolicy::ArchiveAlways => CleanupDisposition::Archived,
            CleanupPolicy::RemoveWhenCleanOtherwiseArchive => {
                if self.environment_is_clean(record)? && !force_archive_dirty {
                    CleanupDisposition::Removed
                } else {
                    CleanupDisposition::Archived
                }
            }
            _ => CleanupDisposition::Kept,
        };

        let retained_path = match disposition {
            CleanupDisposition::Removed => {
                self.remove_environment_path(record)?;
                record.state = EnvironmentState::Removed;
                None
            }
            CleanupDisposition::Archived => {
                record.state = EnvironmentState::Archived;
                Some(record.prepared_path.clone())
            }
            CleanupDisposition::Kept => {
                record.state = EnvironmentState::Ready;
                Some(record.prepared_path.clone())
            }
        };

        Ok(CleanupResult {
            disposition,
            completed_at: Utc::now(),
            retained_path,
            summary: format!(
                "Environment {} cleanup finished with {:?}",
                record.id, disposition
            ),
        })
    }

    fn environment_is_clean(&self, record: &EnvironmentRecord) -> Result<bool, String> {
        match record.spec.execution_mode {
            AgentExecutionMode::GitWorktree => {
                let Some(spec) = record.spec.git_worktree.as_ref() else {
                    return Ok(false);
                };
                GitTools::new(Some(spec.repo_root.clone()))
                    .is_worktree_clean(&spec.worktree_path)
                    .map_err(|error| format!("Failed to inspect git worktree cleanliness: {error}"))
            }
            AgentExecutionMode::IsolatedWorkspace => {
                Ok(is_directory_effectively_empty(&record.prepared_path)?)
            }
            AgentExecutionMode::SharedWorkspace | AgentExecutionMode::Remote => Ok(false),
        }
    }

    fn remove_environment_path(&self, record: &EnvironmentRecord) -> Result<(), String> {
        match record.spec.execution_mode {
            AgentExecutionMode::SharedWorkspace | AgentExecutionMode::Remote => Ok(()),
            AgentExecutionMode::IsolatedWorkspace => {
                if record.prepared_path.exists() {
                    fs::remove_dir_all(&record.prepared_path).map_err(|error| {
                        format!(
                            "Failed to remove isolated environment {}: {error}",
                            record.prepared_path.display()
                        )
                    })?;
                }
                Ok(())
            }
            AgentExecutionMode::GitWorktree => {
                let Some(spec) = record.spec.git_worktree.as_ref() else {
                    return Ok(());
                };
                GitTools::new(Some(spec.repo_root.clone()))
                    .worktree_remove(&spec.worktree_path, true)
                    .map_err(|error| format!("Failed to remove git worktree: {error}"))?;
                GitTools::new(Some(spec.repo_root.clone()))
                    .worktree_prune()
                    .map_err(|error| format!("Failed to prune git worktrees: {error}"))?;
                Ok(())
            }
        }
    }

    fn session_workspace(
        &self,
        workspace_root: &Path,
        session_id: Option<&str>,
    ) -> Result<SessionWorkspace, String> {
        SessionWorkspace::from_directory(
            session_id.unwrap_or("orchestrator"),
            workspace_root.to_path_buf(),
        )
        .map_err(|error| format!("Failed to initialize session workspace: {error}"))
    }
}

pub(super) fn sanitize_component(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => character,
            _ => '-',
        })
        .collect()
}

pub(super) fn default_cleanup_policy(mode: &AgentExecutionMode) -> CleanupPolicy {
    match mode {
        AgentExecutionMode::SharedWorkspace | AgentExecutionMode::Remote => {
            CleanupPolicy::KeepAlways
        }
        AgentExecutionMode::IsolatedWorkspace | AgentExecutionMode::GitWorktree => {
            CleanupPolicy::RemoveWhenCleanOtherwiseArchive
        }
    }
}

fn is_directory_effectively_empty(path: &PathBuf) -> Result<bool, String> {
    if !path.exists() {
        return Ok(true);
    }
    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("Failed to inspect directory {}: {error}", path.display()))?;
    Ok(entries.next().is_none())
}
