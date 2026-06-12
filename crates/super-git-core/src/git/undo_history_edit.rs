use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use crate::config::store::repository_id;
use crate::git::command::Git;
use crate::git::{execute_history_edit, state, undo_registry};
use crate::model::{
    HistoryEditExecutionRecord, HistoryEditUndoToken, Operation, UndoResult,
    HISTORY_EDIT_EXECUTION_RECORD_SCHEMA_VERSION, HISTORY_EDIT_UNDO_TOKEN_SCHEMA_VERSION,
    UNDO_RESULT_SCHEMA_VERSION,
};
use crate::{Result, SuperGitError};

const ACTION_HISTORY_EDIT: &str = "history_edit";
const UNDO_RESTORE_BRANCH_TIP: &str = "restore_branch_tip_snapshot";
const UNDO_RESTORE_BRANCH_TIP_AND_WORKTREE: &str = "restore_branch_tip_and_worktree";

/// Undo a history edit by moving the branch ref back to the pre-execute tip.
/// `restore_branch_tip_snapshot` (tree-preserving ops) moves only the branch
/// pointer: the new and old tips share one tree (the C8-C invariant), so file
/// content cannot change. `restore_branch_tip_and_worktree` (drop, C8-drop-D)
/// is the symmetric inverse of drop execute: it requires a clean working tree,
/// checks ignored-path collisions against the pre-execute tip, restores the
/// ref, then synchronizes the index and working tree back to it.
pub fn undo_history_edit_token(
    current_path: &Path,
    token: HistoryEditUndoToken,
) -> Result<UndoResult> {
    validate_static_token(&token)?;
    let worktree_root = validate_current_repository(current_path, &token)?;
    validate_execution_record(&token)?;

    let git = Git::default();
    let restores_worktree = token.kind == UNDO_RESTORE_BRANCH_TIP_AND_WORKTREE;

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
        return mismatch("operation", "none", operation.as_str());
    }

    // drop undo는 execute와 대칭으로 워킹트리를 동기화하므로, 어떤 write보다
    // 먼저 같은 두 게이트를 통과해야 한다: clean(untracked 포함) 게이트와,
    // 이번에는 pre-execute tip이 tracked로 갖는 경로 기준의 ignored 충돌
    // 게이트(execute가 새 tip 기준으로 막는 것의 역방향 — drop이 지웠던
    // ignored 경로 자리에 사용자가 새 파일을 만들어 뒀을 수 있다).
    if restores_worktree {
        ensure_clean_working_tree(&git, &worktree_root)?;
        ensure_no_ignored_path_collisions(&git, &worktree_root, &token.previous_tip)?;
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

    // drop undo: index/워킹트리를 pre-execute tip으로 동기화한다. ref는 이미
    // 올바르게 복원됐으므로 여기서부터의 실패는 rollback이 아니라 partial
    // failure다(execute와 같은 철학) — record는 provenance로 남는다.
    if restores_worktree {
        if let Err(err) =
            execute_history_edit::sync_working_tree(&git, &worktree_root, &token.previous_tip)
        {
            return Err(sync_partial_failure(&git, &worktree_root, &token, err));
        }
    }

    // 효과가 되돌려졌으니 replay 가드도 푼다: record를 소비해 같은 plan을
    // 다시 실행할 수 있게 한다. plan_id는 상태 기반이라 새 preview를 받아도
    // 같은 record 경로로 떨어지므로, 소비하지 않으면 같은 편집이 그 브랜치에서
    // 영구히 막힌다. best-effort: 제거 실패가 성공한 undo를 실패로 만들면
    // 안 된다.
    let _ = fs::remove_file(&token.execution_record_path);

    Ok(UndoResult {
        schema_version: UNDO_RESULT_SCHEMA_VERSION.to_string(),
        action: ACTION_HISTORY_EDIT.to_string(),
        repository: worktree_root,
        plan_id: token.plan_id.clone(),
        undone: true,
        effects: undo_effects(&token, restores_worktree),
    })
}

fn undo_effects(token: &HistoryEditUndoToken, restores_worktree: bool) -> Vec<String> {
    let mut effects = if restores_worktree {
        vec![
            format!(
                "Restored {} to {} (the pre-execute tip) and synchronized the index and working tree to it.",
                token.branch_ref, token.previous_tip
            ),
            "The dropped commits' patches are back in the final history; their objects were never deleted.".to_string(),
        ]
    } else {
        vec![format!(
            "Restored {} to {} (the pre-execute tip); working-tree files and the index are unchanged.",
            token.branch_ref, token.previous_tip
        )]
    };
    effects.push("Consumed the execution record; the same plan can be executed again.".to_string());
    effects
}

/// drop undo의 clean 게이트: execute와 같은 이유(untracked 포함 — sync가
/// pre-execute tip의 경로를 부활시키며 덮어쓸 수 있다), undo 에러 채널.
fn ensure_clean_working_tree(git: &Git, worktree_root: &Path) -> Result<()> {
    let status = execute_history_edit::read_status_signature(git, worktree_root)?;
    if status.is_empty() {
        return Ok(());
    }
    mismatch(
        "working_tree_clean",
        "clean working tree and index (including untracked)",
        &format!("{} dirty or untracked path(s)", status.lines().count()),
    )
}

fn ensure_no_ignored_path_collisions(
    git: &Git,
    worktree_root: &Path,
    previous_tip: &str,
) -> Result<()> {
    let collisions =
        execute_history_edit::ignored_path_collisions(git, worktree_root, previous_tip)?;
    if collisions.is_empty() {
        return Ok(());
    }
    mismatch(
        "ignored_path_collision",
        "no ignored paths colliding with paths tracked at the pre-execute tip",
        &execute_history_edit::describe_collisions(&collisions),
    )
}

/// ref가 이미 pre-execute tip으로 복원된 뒤의 sync 실패 보고. ref 롤백은
/// 하지 않는다 — ref는 올바르고, 미완인 것은 워킹트리 동기화뿐이다. record는
/// 소비되지 않고 provenance로 남는다(undo가 완결되지 않았으므로).
fn sync_partial_failure(
    git: &Git,
    worktree_root: &Path,
    token: &HistoryEditUndoToken,
    original: SuperGitError,
) -> SuperGitError {
    let observed_tip = read_ref_oid(git, worktree_root, &token.branch_ref)
        .unwrap_or_else(|_| "unreadable".to_string());
    SuperGitError::UndoSyncPartialFailure(Box::new(crate::error::SyncPartialFailureError {
        action: ACTION_HISTORY_EDIT.to_string(),
        message: original.to_string(),
        branch_ref: token.branch_ref.clone(),
        observed_tip,
        sync_completed: false,
        execution_record_path: token.execution_record_path.clone(),
        safe_next: "the branch ref is correctly restored to the pre-execute tip; the index and working tree may be partially synchronized — verify the working-tree state and finish synchronizing to the recorded previous_tip; re-running undo will refuse because the branch no longer points at the post-execute tip".to_string(),
    }))
}

fn validate_static_token(token: &HistoryEditUndoToken) -> Result<()> {
    if token.schema_version != HISTORY_EDIT_UNDO_TOKEN_SCHEMA_VERSION {
        return invalid_token(
            "unsupported_schema_version",
            "history_edit undo supports only super-git.history-edit-undo.v0.1",
        );
    }
    if token.kind != UNDO_RESTORE_BRANCH_TIP && token.kind != UNDO_RESTORE_BRANCH_TIP_AND_WORKTREE {
        return invalid_token(
            "unsupported_undo_kind",
            "history_edit undo supports only restore_branch_tip_snapshot and restore_branch_tip_and_worktree",
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
    undo_registry::validate_execution_record_path(
        &token.git_common_dir,
        &token.execution_record_path,
    )
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use crate::git::command::Git;
    use crate::model::HistoryEditUndoToken;
    use crate::SuperGitError;

    fn run_git(dir: &Path, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .expect("run git")
    }

    fn git(dir: &Path, args: &[&str]) {
        assert!(run_git(dir, args).status.success(), "git {args:?} failed");
    }

    fn rev(dir: &Path, reference: &str) -> String {
        String::from_utf8(run_git(dir, &["rev-parse", reference]).stdout)
            .expect("utf8")
            .trim()
            .to_string()
    }

    #[test]
    fn sync_partial_failure_reports_restored_tip_without_touching_the_record() {
        let tmp = tempfile::tempdir().expect("temp");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a").expect("write");
        git(&repo, &["add", "a.txt"]);
        git(&repo, &["commit", "-q", "-m", "c1"]);
        let previous_tip = rev(&repo, "HEAD");
        std::fs::write(repo.join("b.txt"), "b").expect("write");
        git(&repo, &["add", "b.txt"]);
        git(&repo, &["commit", "-q", "-m", "c2"]);
        let new_tip = rev(&repo, "HEAD");
        // The failure window starts after the CAS restored the ref.
        git(&repo, &["update-ref", "refs/heads/main", &previous_tip]);
        let record_path = repo.join(".git/super-git/executions/record.json");
        std::fs::create_dir_all(record_path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&record_path, "{}").expect("record");
        let token = HistoryEditUndoToken {
            schema_version: "super-git.history-edit-undo.v0.1".to_string(),
            kind: "restore_branch_tip_and_worktree".to_string(),
            repository: repo.clone(),
            action: "history_edit".to_string(),
            plan_id: "sha256:test".to_string(),
            branch_ref: "refs/heads/main".to_string(),
            previous_tip: previous_tip.clone(),
            new_tip,
            git_common_dir: repo.join(".git"),
            family_id: "family".to_string(),
            execution_record_path: record_path.clone(),
            deletes_branch: false,
            deletes_history: false,
        };
        let original = SuperGitError::ExecutePlanInvalid {
            code: "original".to_string(),
            message: "sync broke".to_string(),
        };

        let err = super::sync_partial_failure(&Git::default(), &repo, &token, original);

        assert_eq!(err.code(), "undo_partial_failure");
        match err {
            SuperGitError::UndoSyncPartialFailure(details) => {
                assert_eq!(details.action, "history_edit");
                assert_eq!(
                    details.observed_tip, previous_tip,
                    "the envelope reports the live (already restored) ref tip"
                );
                assert!(!details.sync_completed);
                assert_eq!(details.execution_record_path, PathBuf::from(&record_path));
                assert!(details.safe_next.contains("previous_tip"));
            }
            other => panic!("expected UndoSyncPartialFailure, got {other:?}"),
        }
        assert!(
            record_path.exists(),
            "an unfinished undo must keep the record as provenance"
        );
        assert_eq!(
            rev(&repo, "refs/heads/main"),
            previous_tip,
            "the helper must not roll the ref back"
        );
    }
}
