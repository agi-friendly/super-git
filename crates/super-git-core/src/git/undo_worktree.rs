use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path};

use crate::config::store::SavedRepository;
use crate::git::command::Git;
use crate::git::{state, worktree};
use crate::model::{
    Operation, UndoResult, WorktreeExecutionRecord, WorktreeInfo, WorktreeUndoToken,
    UNDO_RESULT_SCHEMA_VERSION, UNDO_TOKEN_SCHEMA_VERSION,
    WORKTREE_EXECUTION_RECORD_SCHEMA_VERSION,
};
use crate::{Result, SuperGitError};

const ACTION_WORKTREE_CREATE: &str = "worktree_create";
const UNDO_REMOVE_CREATED_WORKTREE: &str = "remove_created_worktree_if_clean";

pub fn undo_worktree_token(current_path: &Path, token: WorktreeUndoToken) -> Result<UndoResult> {
    validate_static_token(&token)?;
    validate_current_repository(current_path, &token)?;
    validate_execution_record(&token)?;

    let git = Git::default();
    let worktrees = worktree::list_worktrees(&token.repository)?;
    let target_index = find_target_worktree_index(&worktrees, &token.target_path)?;
    let target = &worktrees[target_index];
    validate_target_worktree(target_index, target, &token)?;
    validate_target_head_and_ref(&git, &token)?;
    validate_target_is_clean(&token)?;

    let branch_before = read_branch_ref(&git, &token)?;
    git.run_write_in(&token.repository, trusted_remove_args(&token))?;
    verify_removed(&token)?;
    verify_branch_preserved(&git, &token, branch_before.as_deref())?;

    let mut effects = vec![format!(
        "Removed linked worktree at {}.",
        token.target_path.display()
    )];
    if let Some(effect) = remove_created_parent_if_empty(&token)? {
        effects.push(effect);
    }

    Ok(UndoResult {
        schema_version: UNDO_RESULT_SCHEMA_VERSION.to_string(),
        action: ACTION_WORKTREE_CREATE.to_string(),
        repository: token.repository,
        plan_id: token.plan_id,
        undone: true,
        effects,
    })
}

fn validate_static_token(token: &WorktreeUndoToken) -> Result<()> {
    if token.schema_version != UNDO_TOKEN_SCHEMA_VERSION {
        return invalid_token(
            "unsupported_schema_version",
            "undo supports only super-git.undo.v0.1",
        );
    }
    if token.kind != UNDO_REMOVE_CREATED_WORKTREE {
        return invalid_token(
            "unsupported_undo_kind",
            "worktree undo supports only remove_created_worktree_if_clean",
        );
    }
    if token.action != ACTION_WORKTREE_CREATE {
        return invalid_token(
            "unsupported_action",
            "worktree undo supports only worktree_create",
        );
    }
    if token.deletes_branch || token.deletes_history {
        return invalid_token(
            "unsafe_undo_strategy",
            "worktree undo must not delete branches or history",
        );
    }

    for (field, path) in [
        ("repository", &token.repository),
        ("target_path", &token.target_path),
        ("git_common_dir", &token.git_common_dir),
        ("execution_record_path", &token.execution_record_path),
    ] {
        validate_absolute_clean_path(field, path)?;
    }
    if let Some(parent) = &token.created_parent {
        validate_absolute_clean_path("created_parent", parent)?;
    }
    validate_execution_record_path(token)?;
    Ok(())
}

fn validate_current_repository(current_path: &Path, token: &WorktreeUndoToken) -> Result<()> {
    let current = SavedRepository::from_path(current_path)?;
    ensure_match("repository.family_id", &token.family_id, &current.id)?;
    ensure_path_match(
        "repository.git_common_dir",
        &token.git_common_dir,
        &current.git_common_dir,
    )?;
    ensure_path_match("repository", &token.repository, &current.saved_from)
}

fn validate_execution_record(token: &WorktreeUndoToken) -> Result<()> {
    let metadata = fs::symlink_metadata(&token.execution_record_path).map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            SuperGitError::UndoTokenInvalid {
                code: "execution_record_missing".to_string(),
                message: "worktree undo requires the local execution record".to_string(),
            }
        } else {
            err.into()
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return invalid_token(
            "unsafe_execution_record_file",
            "execution record path must be a regular file, not a symlink",
        );
    }

    let bytes = fs::read(&token.execution_record_path)?;
    let record: WorktreeExecutionRecord =
        serde_json::from_slice(&bytes).map_err(|err| SuperGitError::UndoTokenInvalid {
            code: "execution_record_json_invalid".to_string(),
            message: err.to_string(),
        })?;

    if record.schema_version != WORKTREE_EXECUTION_RECORD_SCHEMA_VERSION {
        return invalid_token(
            "unsupported_execution_record_schema",
            "worktree undo supports only super-git.worktree-execution.v0.1",
        );
    }
    if record.status != "completed" {
        return invalid_token(
            "execution_record_incomplete",
            "worktree undo requires a completed execution record",
        );
    }
    if record.action != ACTION_WORKTREE_CREATE {
        return invalid_token(
            "execution_record_action",
            "worktree undo record must describe worktree_create",
        );
    }
    if record.undo_token.as_ref() != Some(token) {
        return invalid_token(
            "execution_record_token_mismatch",
            "execution record undo token must match the provided token",
        );
    }
    ensure_match("execution_record.plan_id", &token.plan_id, &record.plan_id)?;
    ensure_path_match(
        "execution_record.repository",
        &token.repository,
        &record.repository.selected_from,
    )?;
    ensure_path_match(
        "execution_record.git_common_dir",
        &token.git_common_dir,
        &record.repository.git_common_dir,
    )?;
    ensure_match(
        "execution_record.family_id",
        &token.family_id,
        &record.repository.family_id,
    )?;
    ensure_path_match(
        "execution_record.target_path",
        &token.target_path,
        &record.target_path,
    )?;
    ensure_match(
        "execution_record.target_head",
        &token.target_head,
        &record.expected_head,
    )?;
    ensure_optional_match(
        "execution_record.target_branch",
        token.target_branch.as_deref(),
        record.expected_branch.as_deref(),
    )?;
    ensure_optional_path_match(
        "execution_record.created_parent",
        token.created_parent.as_deref(),
        record.created_parent.as_deref(),
    )
}

fn validate_execution_record_path(token: &WorktreeUndoToken) -> Result<()> {
    let execution_dir = token.git_common_dir.join("super-git").join("executions");
    if !token.execution_record_path.starts_with(&execution_dir) {
        return invalid_token(
            "unsafe_execution_record_path",
            "execution record path must stay inside the repository executions directory",
        );
    }

    let relative = token
        .execution_record_path
        .strip_prefix(&execution_dir)
        .map_err(|_| SuperGitError::UndoTokenInvalid {
            code: "unsafe_execution_record_path".to_string(),
            message: "execution record path must be under the executions directory".to_string(),
        })?;
    if relative.components().count() != 1
        || !matches!(relative.components().next(), Some(Component::Normal(_)))
        || token
            .execution_record_path
            .extension()
            .and_then(|ext| ext.to_str())
            != Some("json")
    {
        return invalid_token(
            "unsafe_execution_record_path",
            "execution record path must be a direct .json file inside the executions directory",
        );
    }
    Ok(())
}

fn find_target_worktree_index(worktrees: &[WorktreeInfo], target_path: &Path) -> Result<usize> {
    worktrees
        .iter()
        .position(|worktree| same_path(&worktree.path, target_path))
        .ok_or_else(|| SuperGitError::UndoPreconditionMismatch {
            field: "target_worktree".to_string(),
            expected: "present".to_string(),
            actual: "absent".to_string(),
        })
}

fn validate_target_worktree(
    target_index: usize,
    target: &WorktreeInfo,
    token: &WorktreeUndoToken,
) -> Result<()> {
    ensure_path_match("target_path", &token.target_path, &target.path)?;
    if target_index == 0 || target.bare {
        return mismatch("target_worktree_kind", "linked", "main_or_bare");
    }
    if target.locked {
        return mismatch("target_locked", "false", "true");
    }
    if target.prunable {
        return mismatch("target_prunable", "false", "true");
    }
    Ok(())
}

fn validate_target_head_and_ref(git: &Git, token: &WorktreeUndoToken) -> Result<()> {
    let head = git
        .run_in(
            &token.target_path,
            ["rev-parse", "--verify", "HEAD^{commit}"],
        )?
        .stdout
        .trim()
        .to_string();
    ensure_match("target_head", &token.target_head, &head)?;

    let branch_result = git.try_run_in(&token.target_path, ["symbolic-ref", "-q", "HEAD"])?;
    let current_branch = branch_result
        .success
        .then(|| branch_result.stdout.trim().to_string());
    ensure_optional_match(
        "target_branch",
        token.target_branch.as_deref(),
        current_branch.as_deref(),
    )
}

fn validate_target_is_clean(token: &WorktreeUndoToken) -> Result<()> {
    let repo_state = state::read_state(&token.target_path)?;
    if repo_state.operation != Operation::None {
        return mismatch(
            "target_operation",
            "none",
            operation_name(repo_state.operation),
        );
    }
    if repo_state.working_tree.clean {
        return Ok(());
    }
    mismatch("target_working_tree_clean", "true", "false")
}

fn read_branch_ref(git: &Git, token: &WorktreeUndoToken) -> Result<Option<String>> {
    let Some(branch) = token.target_branch.as_deref() else {
        return Ok(None);
    };
    let output = git.run_in(&token.repository, ["rev-parse", branch])?;
    Ok(Some(output.stdout.trim().to_string()))
}

fn trusted_remove_args(token: &WorktreeUndoToken) -> Vec<OsString> {
    vec![
        OsString::from("worktree"),
        OsString::from("remove"),
        token.target_path.as_os_str().to_os_string(),
    ]
}

fn verify_removed(token: &WorktreeUndoToken) -> Result<()> {
    let worktrees = worktree::list_worktrees(&token.repository)?;
    if worktrees
        .iter()
        .any(|worktree| same_path(&worktree.path, &token.target_path))
    {
        return mismatch("worktree_list.target", "absent", "present");
    }
    Ok(())
}

fn verify_branch_preserved(
    git: &Git,
    token: &WorktreeUndoToken,
    branch_before: Option<&str>,
) -> Result<()> {
    let Some(branch_before) = branch_before else {
        return Ok(());
    };
    let Some(branch) = token.target_branch.as_deref() else {
        return Ok(());
    };
    let output = git.run_in(&token.repository, ["rev-parse", branch])?;
    ensure_match("target_branch_ref", branch_before, output.stdout.trim())
}

fn remove_created_parent_if_empty(token: &WorktreeUndoToken) -> Result<Option<String>> {
    let Some(parent) = &token.created_parent else {
        return Ok(None);
    };
    match fs::remove_dir(parent) {
        Ok(()) => Ok(Some(format!(
            "Removed empty parent directory {}.",
            parent.display()
        ))),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(Some(format!(
            "Parent directory {} was already absent.",
            parent.display()
        ))),
        Err(err) if err.kind() == ErrorKind::DirectoryNotEmpty => Ok(Some(format!(
            "Left parent directory {} because it is not empty.",
            parent.display()
        ))),
        Err(err) => Err(err.into()),
    }
}

fn validate_absolute_clean_path(field: &str, path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return invalid_token("unsafe_path", &format!("{field} must be an absolute path"));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return invalid_token(
            "unsafe_path",
            &format!("{field} must not contain parent/current directory components"),
        );
    }
    Ok(())
}

fn ensure_path_match(field: &str, expected: &Path, actual: &Path) -> Result<()> {
    ensure_match(
        field,
        &expected.display().to_string(),
        &actual.display().to_string(),
    )
}

fn ensure_optional_path_match(
    field: &str,
    expected: Option<&Path>,
    actual: Option<&Path>,
) -> Result<()> {
    let expected = expected.map(|path| path.display().to_string());
    let actual = actual.map(|path| path.display().to_string());
    if expected == actual {
        return Ok(());
    }
    mismatch(
        field,
        expected.as_deref().unwrap_or("null"),
        actual.as_deref().unwrap_or("null"),
    )
}

fn ensure_optional_match(field: &str, expected: Option<&str>, actual: Option<&str>) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    mismatch(field, expected.unwrap_or("null"), actual.unwrap_or("null"))
}

fn ensure_match(field: &str, expected: &str, actual: &str) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    mismatch(field, expected, actual)
}

fn mismatch<T>(field: &str, expected: &str, actual: &str) -> Result<T> {
    Err(SuperGitError::UndoPreconditionMismatch {
        field: field.to_string(),
        expected: expected.to_string(),
        actual: actual.to_string(),
    })
}

fn invalid_token<T>(code: &str, message: &str) -> Result<T> {
    Err(SuperGitError::UndoTokenInvalid {
        code: code.to_string(),
        message: message.to_string(),
    })
}

fn operation_name(operation: Operation) -> &'static str {
    match operation {
        Operation::None => "none",
        Operation::Merging => "merging",
        Operation::Rebasing => "rebasing",
        Operation::Applying => "applying",
        Operation::CherryPicking => "cherry-picking",
        Operation::Reverting => "reverting",
        Operation::Bisecting => "bisecting",
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || std::fs::canonicalize(left)
            .ok()
            .zip(std::fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
}
