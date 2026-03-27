use super::{DelegatedTaskCheckpoint, EnvironmentRecord, SupervisorRun};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn session_key(session_id: Option<&str>) -> &str {
    session_id.unwrap_or("global")
}

fn base_dir(root: &Path) -> PathBuf {
    root.join(".gestura").join("orchestrator")
}

fn session_dir(root: &Path, session_id: Option<&str>) -> PathBuf {
    base_dir(root).join(session_key(session_id))
}

fn runs_dir(root: &Path, session_id: Option<&str>) -> PathBuf {
    session_dir(root, session_id).join("runs")
}

fn environments_dir(root: &Path, session_id: Option<&str>) -> PathBuf {
    session_dir(root, session_id).join("environments")
}

fn checkpoints_dir(root: &Path, session_id: Option<&str>) -> PathBuf {
    session_dir(root, session_id).join("checkpoints")
}

fn load_json_files<T: serde::de::DeserializeOwned>(dir: &Path) -> Vec<T> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                return None;
            }
            let content = fs::read(&path).ok()?;
            serde_json::from_slice::<T>(&content).ok()
        })
        .collect()
}

pub(super) fn persist_run_to_disk(root: &Path, run: &SupervisorRun) -> Result<(), String> {
    let session = run.session_id.as_deref();
    let legacy_dir = session_dir(root, session);
    let dir = runs_dir(root, session);
    fs::create_dir_all(&dir)
        .map_err(|error| format!("Failed to create orchestrator run dir: {error}"))?;
    let path = dir.join(format!("{}.json", run.id));
    let content = serde_json::to_vec_pretty(run)
        .map_err(|error| format!("Failed to serialize supervisor run: {error}"))?;
    fs::write(&path, content)
        .map_err(|error| format!("Failed to persist supervisor run: {error}"))?;

    let legacy_path = legacy_dir.join(format!("{}.json", run.id));
    if legacy_path.exists() {
        let _ = fs::remove_file(legacy_path);
    }
    Ok(())
}

pub(super) async fn persist_run_to_disk_async(
    root: &Path,
    run: &SupervisorRun,
) -> Result<(), String> {
    let root = root.to_path_buf();
    let run = run.clone();
    tokio::task::spawn_blocking(move || persist_run_to_disk(&root, &run))
        .await
        .map_err(|error| format!("Failed to join orchestrator run persistence task: {error}"))?
}

pub(super) fn persist_environment_to_disk(
    root: &Path,
    environment: &EnvironmentRecord,
) -> Result<(), String> {
    let dir = environments_dir(root, environment.spec.session_id.as_deref());
    fs::create_dir_all(&dir)
        .map_err(|error| format!("Failed to create orchestrator environment dir: {error}"))?;
    let path = dir.join(format!("{}.json", environment.id));
    let content = serde_json::to_vec_pretty(environment)
        .map_err(|error| format!("Failed to serialize environment record: {error}"))?;
    fs::write(path, content)
        .map_err(|error| format!("Failed to persist environment record: {error}"))
}

pub(super) async fn persist_environment_to_disk_async(
    root: &Path,
    environment: &EnvironmentRecord,
) -> Result<(), String> {
    let root = root.to_path_buf();
    let environment = environment.clone();
    tokio::task::spawn_blocking(move || persist_environment_to_disk(&root, &environment))
        .await
        .map_err(|error| {
            format!("Failed to join orchestrator environment persistence task: {error}")
        })?
}

pub(super) fn persist_checkpoint_to_disk(
    root: &Path,
    checkpoint: &DelegatedTaskCheckpoint,
) -> Result<(), String> {
    let dir = checkpoints_dir(root, checkpoint.session_id.as_deref());
    fs::create_dir_all(&dir)
        .map_err(|error| format!("Failed to create orchestrator checkpoint dir: {error}"))?;
    let path = dir.join(format!("{}.json", checkpoint.task_id));
    let content = serde_json::to_vec_pretty(checkpoint)
        .map_err(|error| format!("Failed to serialize delegated checkpoint: {error}"))?;
    fs::write(path, content)
        .map_err(|error| format!("Failed to persist delegated checkpoint: {error}"))
}

pub(super) async fn persist_checkpoint_to_disk_async(
    root: &Path,
    checkpoint: &DelegatedTaskCheckpoint,
) -> Result<(), String> {
    let root = root.to_path_buf();
    let checkpoint = checkpoint.clone();
    tokio::task::spawn_blocking(move || persist_checkpoint_to_disk(&root, &checkpoint))
        .await
        .map_err(|error| format!("Failed to join delegated checkpoint persistence task: {error}"))?
}

pub(super) fn load_persisted_runs(root: &Path) -> Vec<SupervisorRun> {
    let Ok(session_dirs) = fs::read_dir(base_dir(root)) else {
        return Vec::new();
    };

    let mut runs = Vec::new();
    for session_dir in session_dirs.flatten() {
        let path = session_dir.path();
        if !path.is_dir() {
            continue;
        }

        let mut seen = HashSet::new();
        let modern_runs_dir = path.join("runs");
        for run in load_json_files::<SupervisorRun>(&modern_runs_dir) {
            seen.insert(run.id.clone());
            runs.push(run);
        }

        for run in load_json_files::<SupervisorRun>(&path) {
            if seen.insert(run.id.clone()) {
                runs.push(run);
            }
        }
    }

    runs
}

pub(super) fn load_persisted_environments(root: &Path) -> Vec<EnvironmentRecord> {
    let Ok(session_dirs) = fs::read_dir(base_dir(root)) else {
        return Vec::new();
    };

    let mut environments = Vec::new();
    for session_dir in session_dirs.flatten() {
        let path = session_dir.path();
        if !path.is_dir() {
            continue;
        }
        environments.extend(load_json_files::<EnvironmentRecord>(
            &path.join("environments"),
        ));
    }
    environments
}

pub(super) fn load_persisted_checkpoints(root: &Path) -> Vec<DelegatedTaskCheckpoint> {
    let Ok(session_dirs) = fs::read_dir(base_dir(root)) else {
        return Vec::new();
    };

    let mut checkpoints = Vec::new();
    for session_dir in session_dirs.flatten() {
        let path = session_dir.path();
        if !path.is_dir() {
            continue;
        }
        checkpoints.extend(load_json_files::<DelegatedTaskCheckpoint>(
            &path.join("checkpoints"),
        ));
    }
    checkpoints
}

pub(super) fn load_persisted_environment_by_id(
    root: &Path,
    environment_id: &str,
) -> Option<EnvironmentRecord> {
    load_persisted_environments(root)
        .into_iter()
        .find(|record| record.id == environment_id)
}

pub(super) async fn load_persisted_environment_by_id_async(
    root: &Path,
    environment_id: &str,
) -> Option<EnvironmentRecord> {
    let root = root.to_path_buf();
    let environment_id = environment_id.to_string();
    let environment_id_for_task = environment_id.clone();
    match tokio::task::spawn_blocking(move || {
        load_persisted_environment_by_id(&root, &environment_id_for_task)
    })
    .await
    {
        Ok(record) => record,
        Err(error) => {
            tracing::warn!(
                environment_id = %environment_id,
                "Failed to join persisted environment lookup task: {error}"
            );
            None
        }
    }
}
