use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use crate::config::store::repository_id;
use crate::git::command::Git;
use crate::git::state;
use crate::model::{
    HistoryEditExecutionRecord, HistoryEditUndoToken, Operation, UndoResult,
    HISTORY_EDIT_EXECUTION_RECORD_SCHEMA_VERSION, HISTORY_EDIT_UNDO_TOKEN_SCHEMA_VERSION,
    UNDO_RESULT_SCHEMA_VERSION,
};
use crate::{Result, SuperGitError};

const ACTION_HISTORY_EDIT: &str = "history_edit";
const UNDO_RESTORE_BRANCH_TIP: &str = "restore_branch_tip_snapshot";

/// Undo a history edit by moving the branch ref back to the pre-execute tip.
/// Only the branch pointer moves: working-tree files, the index, and every
/// other ref are left untouched. Because the new and old tips share one tree
/// (the C8-C invariant), restoring the pointer cannot change file content.
pub fn undo_history_edit_token(
    current_path: &Path,
    token: HistoryEditUndoToken,
) -> Result<UndoResult> {
    validate_static_token(&token)?;
    let worktree_root = validate_current_repository(current_path, &token)?;
    validate_execution_record(&token)?;

    let git = Git::default();

    // The branch must still point exactly where execute left it. A moved tip
    // (new commits, another edit) means undo cannot safely reclaim history.
    let current_tip = read_ref_oid(&git, &worktree_root, &token.branch_ref)?;
    if current_tip != token.new_tip {
        return mismatch(
            "branch_advanced_since_execute",
            &token.new_tip,
            &current_tip,
        );
    }
    // The pre-execute tip must still exist locally (reflog/gc window).
    if !commit_exists(&git, &worktree_root, &token.previous_tip)? {
        return mismatch("previous_tip_reachable", "present", "missing");
    }
    let operation = state::detect_operation(&git, &worktree_root)?;
    if operation != Operation::None {
        return mismatch("operation", "none", operation_name(operation));
    }

    // Compare-and-swap from new_tip back to previous_tip: a concurrent move
    // makes update-ref refuse, so undo never clobbers an unexpected tip. Because
    // the call names exactly one ref, no other ref can change by our hand.
    git.run_write_in(
        &worktree_root,
        [
            "update-ref",
            &token.branch_ref,
            &token.previous_tip,
            &token.new_tip,
        ],
    )
    .map_err(|err| SuperGitError::UndoPreconditionMismatch {
        field: "branch_ref_compare_and_swap".to_string(),
        expected: format!("{}=={}", token.branch_ref, token.new_tip),
        actual: format!("update-ref refused: {err}"),
    })?;

    post_verify(&git, &worktree_root, &token)?;

    Ok(UndoResult {
        schema_version: UNDO_RESULT_SCHEMA_VERSION.to_string(),
        action: ACTION_HISTORY_EDIT.to_string(),
        repository: worktree_root,
        plan_id: token.plan_id,
        undone: true,
        effects: vec![format!(
            "Restored {} to {} (the pre-execute tip); working-tree files and the index are unchanged.",
            token.branch_ref, token.previous_tip
        )],
    })
}

fn validate_static_token(token: &HistoryEditUndoToken) -> Result<()> {
    if token.schema_version != HISTORY_EDIT_UNDO_TOKEN_SCHEMA_VERSION {
        return invalid_token(
            "unsupported_schema_version",
            "history_edit undo supports only super-git.history-edit-undo.v0.1",
        );
    }
    if token.kind != UNDO_RESTORE_BRANCH_TIP {
        return invalid_token(
            "unsupported_undo_kind",
            "history_edit undo supports only restore_branch_tip_snapshot",
        );
    }
    if token.action != ACTION_HISTORY_EDIT {
        return invalid_token(
            "unsupported_action",
            "history_edit undo supports only history_edit",
        );
    }
    if token.deletes_branch || token.deletes_history {
        return invalid_token(
            "unsafe_undo_strategy",
            "history_edit undo must not delete branches or history",
        );
    }
    // Only local branches are eligible; the ref move never touches remotes/tags.
    if !token.branch_ref.starts_with("refs/heads/") {
        return invalid_token(
            "unsafe_branch_ref",
            "history_edit undo only restores local branch refs under refs/heads/",
        );
    }
    validate_oid("previous_tip", &token.previous_tip)?;
    validate_oid("new_tip", &token.new_tip)?;

    for (field, path) in [
        ("repository", &token.repository),
        ("git_common_dir", &token.git_common_dir),
        ("execution_record_path", &token.execution_record_path),
    ] {
        validate_absolute_clean_path(field, path)?;
    }
    validate_execution_record_path(token)?;
    Ok(())
}

fn validate_current_repository(
    current_path: &Path,
    token: &HistoryEditUndoToken,
) -> Result<PathBuf> {
    let git = Git::default();
    let worktree_root = read_path(&git, current_path, ["rev-parse", "--show-toplevel"])?;
    let git_common_dir = read_path(
        &git,
        &worktree_root,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    ensure_match(
        "repository.family_id",
        &token.family_id,
        &repository_id(&git_common_dir),
    )?;
    ensure_path_match(
        "repository.git_common_dir",
        &token.git_common_dir,
        &git_common_dir,
    )?;
    ensure_path_match("repository", &token.repository, &worktree_root)?;
    Ok(worktree_root)
}

fn validate_execution_record(token: &HistoryEditUndoToken) -> Result<()> {
    let metadata = fs::symlink_metadata(&token.execution_record_path).map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            SuperGitError::UndoTokenInvalid {
                code: "execution_record_missing".to_string(),
                message: "history_edit undo requires the local execution record".to_string(),
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
    let record: HistoryEditExecutionRecord =
        serde_json::from_slice(&bytes).map_err(|err| SuperGitError::UndoTokenInvalid {
            code: "execution_record_json_invalid".to_string(),
            message: err.to_string(),
        })?;

    if record.schema_version != HISTORY_EDIT_EXECUTION_RECORD_SCHEMA_VERSION {
        return invalid_token(
            "unsupported_execution_record_schema",
            "history_edit undo supports only super-git.history-edit-execution.v0.1",
        );
    }
    if record.status != "completed" {
        return invalid_token(
            "execution_record_incomplete",
            "history_edit undo requires a completed execution record",
        );
    }
    if record.action != ACTION_HISTORY_EDIT {
        return invalid_token(
            "execution_record_action",
            "history_edit undo record must describe history_edit",
        );
    }
    // The record's embedded token must be exactly this token: provenance proof
    // that this checkout actually executed the matching plan.
    if record.undo_token.as_ref() != Some(token) {
        return invalid_token(
            "execution_record_token_mismatch",
            "execution record undo token must match the provided token",
        );
    }
    ensure_match("execution_record.plan_id", &token.plan_id, &record.plan_id)?;
    ensure_match(
        "execution_record.branch_ref",
        &token.branch_ref,
        &record.branch_ref,
    )?;
    ensure_match(
        "execution_record.previous_tip",
        &token.previous_tip,
        &record.previous_tip,
    )?;
    ensure_match("execution_record.new_tip", &token.new_tip, &record.new_tip)?;
    ensure_path_match(
        "execution_record.repository",
        &token.repository,
        &record.repository.worktree_root,
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
    )
}

fn validate_execution_record_path(token: &HistoryEditUndoToken) -> Result<()> {
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

fn post_verify(git: &Git, worktree_root: &Path, token: &HistoryEditUndoToken) -> Result<()> {
    // The compare-and-swap update-ref names exactly one ref, so the only thing
    // worth confirming is that the branch landed on previous_tip. Diffing the
    // whole ref set would add no safety (we cannot move another ref) while
    // turning an unrelated concurrent ref change, such as a background fetch
    // updating a remote-tracking ref, into a spurious undo failure.
    let tip = read_ref_oid(git, worktree_root, &token.branch_ref)?;
    if tip != token.previous_tip {
        return mismatch("post_verify.branch_tip", &token.previous_tip, &tip);
    }
    Ok(())
}

fn read_ref_oid(git: &Git, worktree_root: &Path, reference: &str) -> Result<String> {
    let result = git.try_run_in(worktree_root, ["rev-parse", "--verify", reference])?;
    if !result.success {
        return mismatch("branch_ref", "present", "absent");
    }
    Ok(result.stdout.trim().to_string())
}

fn commit_exists(git: &Git, worktree_root: &Path, oid: &str) -> Result<bool> {
    let result = git.try_run_in(
        worktree_root,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{oid}^{{commit}}"),
        ],
    )?;
    Ok(result.success)
}

fn read_path<const N: usize>(git: &Git, path: &Path, args: [&str; N]) -> Result<PathBuf> {
    git.run_path_in(path, args)
}

fn validate_oid(field: &str, oid: &str) -> Result<()> {
    if oid.len() < 4 || oid.len() > 64 || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return invalid_token(
            "unsafe_commit_id",
            &format!("{field} must be a hex object id"),
        );
    }
    Ok(())
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
