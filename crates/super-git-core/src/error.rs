use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, SuperGitError>;

#[derive(Debug, Error)]
pub enum SuperGitError {
    #[error("config directory is not available on this platform")]
    ConfigDirectoryUnavailable,

    #[error("SUPER_GIT_HOME is set but empty")]
    EmptySuperGitHome,

    #[error("SUPER_GIT_HOME must be an absolute path: {0}")]
    RelativeSuperGitHome(PathBuf),

    #[error("invalid config schema version: {0}")]
    InvalidConfigSchemaVersion(String),

    #[error(
        "unsupported_config_schema: unsupported config schema version: {version} (current: {current})"
    )]
    UnsupportedConfigSchemaVersion { version: u64, current: u32 },

    #[error("config validation failed: {code} ({field}: {message})")]
    ConfigValidationFailed {
        field: String,
        code: String,
        message: String,
    },

    #[error("repository_not_found: no saved repository matches {target}")]
    RepositoryNotFound { target: String },

    #[error("ambiguous_repository_target: {target} matches multiple repositories: {matches:?}")]
    AmbiguousRepositoryTarget {
        target: String,
        matches: Vec<String>,
    },

    #[error("path does not exist: {0}")]
    PathDoesNotExist(PathBuf),

    #[error("path is not a directory: {0}")]
    PathIsNotDirectory(PathBuf),

    #[error("path is not a Git repository or inside a Git work tree: {0}")]
    NotGitRepository(PathBuf),

    #[error("git command failed: git {args:?} (status: {status:?})\n{stderr}")]
    GitCommandFailed {
        args: Vec<String>,
        status: Option<i32>,
        stderr: String,
    },

    #[error("preview precondition failed for {action}: {code} ({message})")]
    PreviewPreconditionFailed {
        action: String,
        code: String,
        message: String,
    },

    #[error("execute plan invalid: {code} ({message})")]
    ExecutePlanInvalid { code: String, message: String },

    #[error(
        "execute precondition mismatch: {field} expected {expected} but current state is {actual}"
    )]
    ExecutePreconditionMismatch {
        field: String,
        expected: String,
        actual: String,
    },

    #[error(
        "execute rollback failed after post-write failure: original error: {original_error}; rollback error: {rollback_error}"
    )]
    ExecuteRollbackFailed {
        original_error: String,
        rollback_error: String,
    },

    #[error("undo token invalid: {code} ({message})")]
    UndoTokenInvalid { code: String, message: String },

    #[error(
        "undo precondition mismatch: {field} expected {expected} but current state is {actual}"
    )]
    UndoPreconditionMismatch {
        field: String,
        expected: String,
        actual: String,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
