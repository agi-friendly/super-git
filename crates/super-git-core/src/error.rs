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

    #[error(
        "execute partial failure for {action}: {message}; execution_record_path={execution_record_path}; target_path={target_path}; target_path_exists={target_path_exists}; worktree_list_entry_present={worktree_list_entry_present}; cleanup=safe_next:inspect_cleanup_record"
    )]
    ExecutePartialFailure {
        action: String,
        message: String,
        execution_record_path: PathBuf,
        target_path: PathBuf,
        target_path_exists: bool,
        worktree_list_entry_present: bool,
    },

    #[error(transparent)]
    ExecuteSyncPartialFailure(Box<ExecuteSyncPartialFailureError>),

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

/// ref가 이미 새 tip으로 움직인 뒤(sync/record 단계)의 실패 페이로드.
/// SuperGitError 전체가 커지지 않도록 박싱해 담는다(clippy::result_large_err).
#[derive(Debug, Error)]
#[error(
    "execute partial failure for {action}: {message}; branch_ref={branch_ref}; observed_tip={observed_tip}; sync_completed={sync_completed}; execution_record_path={execution_record_path}; safe_next={safe_next}"
)]
pub struct ExecuteSyncPartialFailureError {
    pub action: String,
    pub message: String,
    pub branch_ref: String,
    pub observed_tip: String,
    pub sync_completed: bool,
    pub execution_record_path: PathBuf,
    pub safe_next: String,
}

impl SuperGitError {
    /// Stable machine-readable error code for the JSON envelope, so agents can
    /// branch on `error.code` instead of regexing English prose. Variants that
    /// already carry a domain code (preview/execute/undo contract errors)
    /// surface that inner code directly.
    pub fn code(&self) -> &str {
        match self {
            Self::ConfigDirectoryUnavailable => "config_directory_unavailable",
            Self::EmptySuperGitHome => "super_git_home_empty",
            Self::RelativeSuperGitHome(_) => "super_git_home_relative",
            Self::InvalidConfigSchemaVersion(_) => "config_schema_invalid",
            Self::UnsupportedConfigSchemaVersion { .. } => "unsupported_config_schema",
            Self::ConfigValidationFailed { code, .. } => code,
            Self::RepositoryNotFound { .. } => "repository_not_found",
            Self::AmbiguousRepositoryTarget { .. } => "ambiguous_repository_target",
            Self::PathDoesNotExist(_) => "path_does_not_exist",
            Self::PathIsNotDirectory(_) => "path_is_not_directory",
            Self::NotGitRepository(_) => "not_git_repository",
            Self::GitCommandFailed { .. } => "git_command_failed",
            Self::PreviewPreconditionFailed { code, .. } => code,
            Self::ExecutePlanInvalid { code, .. } => code,
            Self::ExecutePreconditionMismatch { .. } => "execute_precondition_mismatch",
            Self::ExecuteRollbackFailed { .. } => "execute_rollback_failed",
            Self::ExecutePartialFailure { .. } => "execute_partial_failure",
            // worktree_create의 partial failure와 같은 코드: 에이전트 계약은
            // "ref/대상은 이미 움직였고 자동 복구가 없다"는 한 가지다. 구분은
            // action 필드가 한다.
            Self::ExecuteSyncPartialFailure { .. } => "execute_partial_failure",
            Self::UndoTokenInvalid { code, .. } => code,
            Self::UndoPreconditionMismatch { .. } => "undo_precondition_mismatch",
            Self::Io(_) => "io_error",
            Self::Json(_) => "json_error",
        }
    }
}
