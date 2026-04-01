//! Git repository operations tool
//!
//! Provides git operations with structured output.

use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Git repository status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatus {
    pub branch: String,
    pub is_clean: bool,
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    pub untracked: Vec<PathBuf>,
}

/// A file change in git
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: PathBuf,
    pub status: ChangeStatus,
}

/// Type of change
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Unknown,
}

/// Git diff result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDiff {
    pub files: Vec<FileDiff>,
    pub stats: DiffStats,
}

/// Diff for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: PathBuf,
    pub hunks: Vec<DiffHunk>,
}

/// A hunk in a diff
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// A line in a diff
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLine {
    pub line_type: DiffLineType,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiffLineType {
    Context,
    Added,
    Removed,
}

/// Diff statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffStats {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}

/// Git commit info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

/// Git branch info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub is_remote: bool,
    pub upstream: Option<String>,
}

/// Information about a git worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitWorktreeInfo {
    /// Filesystem path of the worktree.
    pub path: PathBuf,
    /// Branch name associated with the worktree, when attached.
    pub branch: Option<String>,
    /// HEAD commit for the worktree.
    pub head: Option<String>,
    /// Whether the worktree is bare.
    pub is_bare: bool,
    /// Whether the worktree is detached.
    pub is_detached: bool,
}

/// Git operations service
pub struct GitTools {
    work_dir: Option<PathBuf>,
}

impl Default for GitTools {
    fn default() -> Self {
        Self::new(None)
    }
}

impl GitTools {
    pub fn new(work_dir: Option<PathBuf>) -> Self {
        Self { work_dir }
    }

    /// Set working directory
    pub fn with_work_dir(mut self, dir: PathBuf) -> Self {
        self.work_dir = Some(dir);
        self
    }

    fn run_git(&self, args: &[&str]) -> Result<String> {
        self.run_git_in(self.work_dir.as_deref(), args)
    }

    fn run_git_in(&self, dir: Option<&Path>, args: &[&str]) -> Result<String> {
        let mut cmd = Command::new("git");
        if let Some(dir) = dir {
            cmd.current_dir(dir);
        }
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

        let output = cmd.output().map_err(AppError::Io)?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(AppError::Io(std::io::Error::other(
                String::from_utf8_lossy(&output.stderr).to_string(),
            )))
        }
    }

    fn run_git_status_only(&self, dir: Option<&Path>, args: &[&str]) -> Result<bool> {
        let mut cmd = Command::new("git");
        if let Some(dir) = dir {
            cmd.current_dir(dir);
        }
        let status = cmd.args(args).status().map_err(AppError::Io)?;
        Ok(status.success())
    }

    /// Resolve the repository top-level path.
    pub fn rev_parse_toplevel(&self) -> Result<PathBuf> {
        self.run_git(&["rev-parse", "--show-toplevel"])
            .map(PathBuf::from)
    }

    /// Return the current branch name.
    pub fn current_branch(&self) -> Result<String> {
        self.run_git(&["branch", "--show-current"])
    }

    /// Check whether a local branch exists.
    pub fn branch_exists(&self, branch: &str) -> Result<bool> {
        self.run_git_status_only(
            self.work_dir.as_deref(),
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
    }

    /// Check whether the current working directory is inside a git repository.
    pub fn path_is_git_repo(&self) -> Result<bool> {
        self.run_git_status_only(
            self.work_dir.as_deref(),
            &["rev-parse", "--is-inside-work-tree"],
        )
    }

    /// List all worktrees for the repository.
    pub fn worktree_list(&self) -> Result<Vec<GitWorktreeInfo>> {
        let output = self.run_git(&["worktree", "list", "--porcelain"])?;
        parse_worktree_list(&output)
    }

    /// Create a worktree for an existing or new branch.
    pub fn worktree_add(
        &self,
        path: &Path,
        branch: &str,
        base_branch: &str,
        create_branch: bool,
    ) -> Result<GitWorktreeInfo> {
        let path_str = path.to_string_lossy().to_string();
        let branch_exists = self.branch_exists(branch)?;

        let mut args: Vec<&str> = vec!["worktree", "add"];
        if create_branch && !branch_exists {
            args.push("-b");
            args.push(branch);
            args.push(&path_str);
            args.push(base_branch);
        } else {
            args.push(&path_str);
            args.push(branch);
        }

        self.run_git(&args)?;
        let desired_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.worktree_list()?
            .into_iter()
            .find(|info| {
                info.path == desired_path
                    || info.path.canonicalize().ok().as_ref() == Some(&desired_path)
            })
            .ok_or_else(|| {
                AppError::Io(std::io::Error::other(
                    "Created worktree was not discoverable",
                ))
            })
    }

    /// Remove a worktree path.
    pub fn worktree_remove(&self, path: &Path, force: bool) -> Result<()> {
        let path_str = path.to_string_lossy().to_string();
        if force {
            self.run_git(&["worktree", "remove", "--force", &path_str])?;
        } else {
            self.run_git(&["worktree", "remove", &path_str])?;
        }
        Ok(())
    }

    /// Prune stale worktree metadata.
    pub fn worktree_prune(&self) -> Result<()> {
        self.run_git(&["worktree", "prune"])?;
        Ok(())
    }

    /// Check whether a worktree has uncommitted changes.
    pub fn is_worktree_clean(&self, path: &Path) -> Result<bool> {
        let status = self.run_git_in(Some(path), &["status", "--porcelain"])?;
        Ok(status.trim().is_empty())
    }

    /// Get repository status
    pub fn status(&self) -> Result<GitStatus> {
        let branch = self.run_git(&["branch", "--show-current"])?;
        let porcelain = self.run_git(&["status", "--porcelain"])?;

        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        let mut untracked = Vec::new();

        for line in porcelain.lines() {
            if line.len() < 3 {
                continue;
            }
            let index_status = line.chars().next().unwrap_or(' ');
            let worktree_status = line.chars().nth(1).unwrap_or(' ');
            let path = PathBuf::from(&line[3..]);

            if index_status == '?' {
                untracked.push(path);
            } else {
                if index_status != ' ' {
                    staged.push(FileChange {
                        path: path.clone(),
                        status: parse_status(index_status),
                    });
                }
                if worktree_status != ' ' {
                    unstaged.push(FileChange {
                        path,
                        status: parse_status(worktree_status),
                    });
                }
            }
        }

        Ok(GitStatus {
            branch,
            is_clean: staged.is_empty() && unstaged.is_empty() && untracked.is_empty(),
            staged,
            unstaged,
            untracked,
        })
    }

    /// Get diff (staged or unstaged)
    pub fn diff(&self, staged: bool, path: Option<&Path>) -> Result<String> {
        let mut args = vec!["diff"];
        if staged {
            args.push("--staged");
        }
        if let Some(p) = path {
            args.push("--");
            let path_str = p.to_string_lossy();
            // We need to handle the lifetime properly
            return self.run_git(
                &[
                    "diff",
                    if staged { "--staged" } else { "" },
                    "--",
                    &path_str,
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>(),
            );
        }
        self.run_git(&args)
    }

    /// Get commit log
    pub fn log(&self, limit: Option<usize>, path: Option<&Path>) -> Result<Vec<CommitInfo>> {
        let limit_str = limit.unwrap_or(10).to_string();
        let format = "--format=%H|%h|%an|%ai|%s";

        let output = if let Some(p) = path {
            self.run_git(&["log", "-n", &limit_str, format, "--", &p.to_string_lossy()])?
        } else {
            self.run_git(&["log", "-n", &limit_str, format])?
        };

        let commits = output
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(5, '|').collect();
                if parts.len() >= 5 {
                    Some(CommitInfo {
                        hash: parts[0].to_string(),
                        short_hash: parts[1].to_string(),
                        author: parts[2].to_string(),
                        date: parts[3].to_string(),
                        message: parts[4].to_string(),
                    })
                } else {
                    None
                }
            })
            .collect();

        Ok(commits)
    }

    /// Create a commit
    pub fn commit(&self, message: &str, all: bool) -> Result<CommitInfo> {
        if all {
            self.run_git(&["add", "-A"])?;
        }

        self.run_git(&["commit", "-m", message])?;

        // Get the commit we just made
        let commits = self.log(Some(1), None)?;
        commits
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Io(std::io::Error::other("Failed to get commit info")))
    }

    /// Undo last commit (soft reset)
    pub fn undo(&self, soft: bool) -> Result<String> {
        let flag = if soft { "--soft" } else { "--mixed" };
        self.run_git(&["reset", flag, "HEAD~1"])
    }

    /// List branches
    pub fn branches(&self, all: bool) -> Result<Vec<BranchInfo>> {
        let args = if all {
            vec!["branch", "-a"]
        } else {
            vec!["branch"]
        };
        let output = self.run_git(&args)?;

        let branches = output
            .lines()
            .map(|line| {
                let is_current = line.starts_with('*');
                let name = line.trim_start_matches(['*', ' ']).to_string();
                let is_remote = name.starts_with("remotes/");
                BranchInfo {
                    name,
                    is_current,
                    is_remote,
                    upstream: None,
                }
            })
            .collect();

        Ok(branches)
    }

    /// Checkout branch or file
    pub fn checkout(&self, target: &str, create: bool) -> Result<String> {
        if create {
            self.run_git(&["checkout", "-b", target])
        } else {
            self.run_git(&["checkout", target])
        }
    }

    /// Stash changes
    pub fn stash(&self, pop: bool, message: Option<&str>) -> Result<String> {
        if pop {
            self.run_git(&["stash", "pop"])
        } else if let Some(msg) = message {
            self.run_git(&["stash", "push", "-m", msg])
        } else {
            self.run_git(&["stash"])
        }
    }

    /// Get blame for file
    pub fn blame(&self, path: &Path, line_range: Option<(usize, usize)>) -> Result<String> {
        if let Some((start, end)) = line_range {
            self.run_git(&[
                "blame",
                "-L",
                &format!("{},{}", start, end),
                &path.to_string_lossy(),
            ])
        } else {
            self.run_git(&["blame", &path.to_string_lossy()])
        }
    }
}

fn parse_status(c: char) -> ChangeStatus {
    match c {
        'A' => ChangeStatus::Added,
        'M' => ChangeStatus::Modified,
        'D' => ChangeStatus::Deleted,
        'R' => ChangeStatus::Renamed,
        'C' => ChangeStatus::Copied,
        _ => ChangeStatus::Unknown,
    }
}

fn parse_worktree_list(output: &str) -> Result<Vec<GitWorktreeInfo>> {
    let mut worktrees = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch: Option<String> = None;
    let mut current_head: Option<String> = None;
    let mut is_bare = false;
    let mut is_detached = false;

    let flush = |worktrees: &mut Vec<GitWorktreeInfo>,
                 current_path: &mut Option<PathBuf>,
                 current_branch: &mut Option<String>,
                 current_head: &mut Option<String>,
                 is_bare: &mut bool,
                 is_detached: &mut bool| {
        if let Some(path) = current_path.take() {
            worktrees.push(GitWorktreeInfo {
                path,
                branch: current_branch.take(),
                head: current_head.take(),
                is_bare: *is_bare,
                is_detached: *is_detached,
            });
        }
        *is_bare = false;
        *is_detached = false;
    };

    for line in output.lines() {
        if line.trim().is_empty() {
            flush(
                &mut worktrees,
                &mut current_path,
                &mut current_branch,
                &mut current_head,
                &mut is_bare,
                &mut is_detached,
            );
            continue;
        }

        if let Some(value) = line.strip_prefix("worktree ") {
            flush(
                &mut worktrees,
                &mut current_path,
                &mut current_branch,
                &mut current_head,
                &mut is_bare,
                &mut is_detached,
            );
            current_path = Some(PathBuf::from(value.trim()));
        } else if let Some(value) = line.strip_prefix("HEAD ") {
            current_head = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("branch refs/heads/") {
            current_branch = Some(value.trim().to_string());
        } else if line.trim() == "bare" {
            is_bare = true;
        } else if line.trim() == "detached" {
            is_detached = true;
        }
    }

    flush(
        &mut worktrees,
        &mut current_path,
        &mut current_branch,
        &mut current_head,
        &mut is_bare,
        &mut is_detached,
    );

    if worktrees.is_empty() && !output.trim().is_empty() {
        return Err(AppError::Io(std::io::Error::other(
            "Failed to parse git worktree list output",
        )));
    }

    Ok(worktrees)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[cfg(not(target_os = "windows"))]
    fn init_test_repo() -> tempfile::TempDir {
        let temp = tempdir().unwrap();
        let repo = temp.path();

        let git = GitTools::new(Some(repo.to_path_buf()));
        git.run_git(&["init", "-b", "main"]).unwrap();
        git.run_git(&["config", "user.email", "test@example.com"])
            .unwrap();
        git.run_git(&["config", "user.name", "Test User"]).unwrap();
        fs::write(repo.join("README.md"), "hello\n").unwrap();
        git.run_git(&["add", "README.md"]).unwrap();
        git.run_git(&["commit", "-m", "initial"]).unwrap();
        temp
    }

    #[test]
    fn test_parse_status() {
        assert!(matches!(parse_status('A'), ChangeStatus::Added));
        assert!(matches!(parse_status('M'), ChangeStatus::Modified));
        assert!(matches!(parse_status('D'), ChangeStatus::Deleted));
        assert!(matches!(parse_status('R'), ChangeStatus::Renamed));
        assert!(matches!(parse_status('C'), ChangeStatus::Copied));
        assert!(matches!(parse_status('X'), ChangeStatus::Unknown));
    }

    #[test]
    fn test_git_tools_new() {
        let tools = GitTools::new(None);
        assert!(tools.work_dir.is_none());
    }

    #[test]
    fn test_git_tools_with_work_dir() {
        let tools = GitTools::new(Some(PathBuf::from("/tmp/test")));
        assert_eq!(tools.work_dir, Some(PathBuf::from("/tmp/test")));
    }

    #[test]
    fn test_status_in_git_repo() {
        // This test runs in the gestura-app repo
        let tools = GitTools::new(None);
        let status = tools.status();
        // Should succeed since we're in a git repo
        assert!(status.is_ok());
        let status = status.unwrap();
        assert!(!status.branch.is_empty());
    }

    #[test]
    fn test_log_in_git_repo() {
        let tools = GitTools::new(None);
        let log = tools.log(Some(5), None);
        assert!(log.is_ok());
        let log = log.unwrap();
        assert!(!log.is_empty());
    }

    #[test]
    fn test_branches() {
        let tools = GitTools::new(None);
        let branches = tools.branches(false);
        assert!(branches.is_ok());
        let branches = branches.unwrap();
        // Should have at least one branch
        assert!(!branches.is_empty());
    }

    #[test]
    fn test_parse_worktree_list() {
        let parsed = parse_worktree_list(
            "worktree /tmp/repo\nHEAD abc123\nbranch refs/heads/main\n\nworktree /tmp/repo-feature\nHEAD def456\nbranch refs/heads/feature\n\n",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].branch.as_deref(), Some("main"));
        assert_eq!(parsed[1].branch.as_deref(), Some("feature"));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn test_worktree_lifecycle() {
        let temp = init_test_repo();
        let worktree_parent = tempdir().unwrap();
        let repo = temp.path();
        let tools = GitTools::new(Some(repo.to_path_buf()));
        let worktree_path = worktree_parent.path().join("feature-worktree");

        let info = tools
            .worktree_add(&worktree_path, "gestura/test-feature", "main", true)
            .unwrap();
        let normalized_worktree_path = worktree_path.canonicalize().unwrap();
        assert_eq!(info.path, normalized_worktree_path);
        assert_eq!(info.branch.as_deref(), Some("gestura/test-feature"));
        assert!(
            tools
                .worktree_list()
                .unwrap()
                .iter()
                .any(|entry| entry.path == normalized_worktree_path)
        );
        assert!(tools.is_worktree_clean(&worktree_path).unwrap());

        fs::write(worktree_path.join("README.md"), "changed\n").unwrap();
        assert!(!tools.is_worktree_clean(&worktree_path).unwrap());

        tools.worktree_remove(&worktree_path, true).unwrap();
        tools.worktree_prune().unwrap();
        assert!(!worktree_path.exists());
    }
}
