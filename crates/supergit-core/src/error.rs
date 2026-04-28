use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, SuperGitError>;

#[derive(Debug, Error)]
pub enum SuperGitError {
    #[error("config directory is not available on this platform")]
    ConfigDirectoryUnavailable,

    #[error("path does not exist: {0}")]
    PathDoesNotExist(PathBuf),

    #[error("path is not a Git repository or inside a Git work tree: {0}")]
    NotGitRepository(PathBuf),

    #[error("git command failed: git {args:?} (status: {status:?})\n{stderr}")]
    GitCommandFailed {
        args: Vec<String>,
        status: Option<i32>,
        stderr: String,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
