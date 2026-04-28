use std::fs;
use std::path::{Path, PathBuf};

use crate::git::command::Git;
use crate::{Result, SuperGitError};

pub fn normalize_existing_path(path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        return Err(SuperGitError::PathDoesNotExist(path.to_path_buf()));
    }

    Ok(fs::canonicalize(path)?)
}

pub fn validate_repository_path(path: &Path) -> Result<PathBuf> {
    let normalized = normalize_existing_path(path)?;

    if is_git_repository_or_worktree(&normalized)? {
        Ok(normalized)
    } else {
        Err(SuperGitError::NotGitRepository(normalized))
    }
}

pub fn is_git_repository_or_worktree(path: &Path) -> Result<bool> {
    let git = Git::default();

    let work_tree = git.try_run_in(path, &["rev-parse", "--is-inside-work-tree"])?;
    if work_tree.success && work_tree.stdout.trim() == "true" {
        return Ok(true);
    }

    let bare_repo = git.try_run_in(path, &["rev-parse", "--is-bare-repository"])?;
    Ok(bare_repo.success && bare_repo.stdout.trim() == "true")
}
