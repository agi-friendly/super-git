use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::git::command::Git;
use crate::git::{execute, worktree, worktree_remove};
use crate::model::{
    ExecuteResult, UnavailableUndoStrategy, WorktreeRemoveExecutionRecord, WorktreeRemovePlan,
    WorktreeRemoveRepository, WorktreeRemoveTarget, WorktreeRemoveTargetState,
    EXECUTE_SCHEMA_VERSION, WORKTREE_REMOVE_EXECUTION_RECORD_SCHEMA_VERSION,
};
use crate::{Result, SuperGitError};

const ACTION_WORKTREE_REMOVE: &str = "worktree_remove";

pub fn execute_worktree_remove_plan(
    current_path: &Path,
    plan: WorktreeRemovePlan,
) -> Result<ExecuteResult> {
    validate_execution_contract(&plan)?;

    let git = Git::default();
    let execution_context = execution_context(&plan);
    let fresh = worktree_remove::scan_worktree_remove_target(
        current_path,
        &plan.target.worktree_list_path,
    )?;
    validate_fresh_scan(&plan, &fresh)?;

    let execution_record_path = execution_record_path(&plan);
    let trusted_args = trusted_git_args(&plan.target.worktree_list_path);

    // Read the branch oid before writing the intent record. This is read-only,
    // so a failure here is a clean precondition error rather than a partial
    // failure that also orphans an intent record.
    let branch_ref = plan
        .target
        .branch
        .as_deref()
        .map(|branch| branch.strip_prefix("refs/heads/").unwrap_or(branch));
    let branch_oid_before = match branch_ref {
        Some(branch) => Some(read_branch_oid(&git, execution_context, branch)?),
        None => None,
    };

    let intent = record_for(
        "intent",
        &plan,
        trusted_args.iter().map(display_arg).collect(),
    );
    write_record_create_new(&execution_record_path, &intent)?;

    if let Err(err) = git.run_write_in(execution_context, trusted_args) {
        return Err(partial_failure(
            err,
            &execution_record_path,
            execution_context,
            &plan.target.worktree_list_path,
        ));
    }

    post_verify(&plan, execution_context).map_err(|err| {
        partial_failure(
            err,
            &execution_record_path,
            execution_context,
            &plan.target.worktree_list_path,
        )
    })?;
    if let Some((branch, expected_oid)) = branch_ref.zip(branch_oid_before.as_ref()) {
        let current_oid = read_branch_oid(&git, execution_context, branch).map_err(|err| {
            partial_failure(
                err,
                &execution_record_path,
                execution_context,
                &plan.target.worktree_list_path,
            )
        })?;
        if current_oid != *expected_oid {
            return Err(partial_failure(
                format!(
                    "branch ref refs/heads/{branch} changed during worktree remove: expected {expected_oid}, got {current_oid}"
                ),
                &execution_record_path,
                execution_context,
                &plan.target.worktree_list_path,
            ));
        }
    }

    let completed = WorktreeRemoveExecutionRecord {
        status: "completed".to_string(),
        ..intent
    };
    write_record_replace(&execution_record_path, &completed).map_err(|err| {
        partial_failure(
            err,
            &execution_record_path,
            execution_context,
            &plan.target.worktree_list_path,
        )
    })?;
    let result_repository = execution_context.to_path_buf();

    Ok(ExecuteResult {
        schema_version: EXECUTE_SCHEMA_VERSION.to_string(),
        plan_id: plan.plan_id,
        action: ACTION_WORKTREE_REMOVE.to_string(),
        repository: result_repository,
        executed: true,
        effects: vec![format!(
            "Removed linked worktree at {} without deleting branch refs or history.",
            plan.target.worktree_list_path.display()
        )],
        undo_token: None,
    })
}

fn execution_context(plan: &WorktreeRemovePlan) -> &Path {
    plan.repository
        .main_worktree
        .as_deref()
        .unwrap_or(&plan.repository.git_common_dir)
}

fn validate_execution_contract(plan: &WorktreeRemovePlan) -> Result<()> {
    if plan.execution.status != "preview_only" {
        return invalid_plan(
            "not_executable",
            "worktree_remove execute requires execution.status=preview_only",
        );
    }
    if !plan.execution.execute_supported
        || plan.execution.future_execute_eligibility != "needs_human_confirmation"
        || plan.execution.raw_git_allowed
        || !plan.execution.blocked_reasons.is_empty()
    {
        return invalid_plan(
            "unsupported_execution_contract",
            "worktree_remove execute requires a confirmed, unblocked super-git-only preview",
        );
    }
    if plan.risk.severity != "high"
        || plan.risk.reversibility != "not_automatically_reversible"
        || !plan.risk.requires_human_confirmation
    {
        return invalid_plan(
            "unsupported_risk",
            "worktree_remove execute requires high not_automatically_reversible risk with human confirmation",
        );
    }
    if plan
        .preconditions
        .iter()
        .any(|precondition| precondition.status != "passed")
    {
        return invalid_plan(
            "preconditions_not_passed",
            "worktree_remove execute requires all preview preconditions to be passed",
        );
    }
    if plan.target.kind != "linked" {
        return invalid_plan(
            "target_not_linked_worktree",
            "worktree_remove execute supports only linked worktrees",
        );
    }
    if plan.target.detached
        || plan.target.locked
        || plan.target.prunable
        || plan.target.is_current_worktree
        || plan.target.has_submodules
    {
        return invalid_plan(
            "unsupported_target_state",
            "worktree_remove execute requires a clean linked target that is not current, detached, locked, prunable, or a submodule container",
        );
    }
    if plan.target_state.operation != crate::model::Operation::None
        || !plan.target_state.working_tree.clean
        || plan.target_state.working_tree.staged != 0
        || plan.target_state.working_tree.unstaged != 0
        || plan.target_state.working_tree.untracked != 0
        || plan.target_state.working_tree.ignored != 0
        || plan.target_state.working_tree.conflict_count != 0
    {
        return invalid_plan(
            "target_not_clean",
            "worktree_remove execute requires a clean target snapshot",
        );
    }
    Ok(())
}

fn validate_fresh_scan(
    plan: &WorktreeRemovePlan,
    fresh: &worktree_remove::WorktreeRemoveScan,
) -> Result<()> {
    if !fresh.blocks.is_empty() {
        let codes = fresh
            .blocks
            .iter()
            .map(|block| block.code.as_str())
            .collect::<Vec<_>>()
            .join(",");
        return mismatch("fresh_target_blocks", "none", &codes);
    }
    if fresh.execution_status != "preview_only" {
        return mismatch(
            "fresh_target_execution_status",
            "preview_only",
            &fresh.execution_status,
        );
    }

    ensure_match(
        "repository.family_id",
        &plan.repository.family_id,
        &fresh.repository.family_id,
    )?;
    ensure_path_match(
        "repository.git_common_dir",
        &plan.repository.git_common_dir,
        &fresh.repository.git_common_dir,
    )?;
    ensure_path_option_match(
        "repository.main_worktree",
        &plan.repository.main_worktree,
        &fresh.repository.main_worktree,
    )?;
    ensure_path_match(
        "target.worktree_list_path",
        &plan.target.worktree_list_path,
        &fresh.target.worktree_list_path,
    )?;
    ensure_path_match(
        "target.canonical_path",
        &plan.target.canonical_path,
        &fresh.target.canonical_path,
    )?;
    ensure_match("target.kind", &plan.target.kind, &fresh.target.kind)?;
    ensure_path_option_match(
        "target.git_common_dir",
        &plan.target.git_common_dir,
        &fresh.target.git_common_dir,
    )?;
    ensure_path_option_match(
        "target.worktree_git_dir",
        &plan.target.worktree_git_dir,
        &fresh.target.worktree_git_dir,
    )?;
    ensure_option_match("target.head", &plan.target.head, &fresh.target.head)?;
    ensure_option_match("target.branch", &plan.target.branch, &fresh.target.branch)?;
    ensure_bool(
        "target.detached",
        plan.target.detached,
        fresh.target.detached,
    )?;
    ensure_bool("target.locked", plan.target.locked, fresh.target.locked)?;
    ensure_bool(
        "target.prunable",
        plan.target.prunable,
        fresh.target.prunable,
    )?;
    ensure_bool(
        "target.is_current_worktree",
        plan.target.is_current_worktree,
        fresh.target.is_current_worktree,
    )?;
    ensure_bool(
        "target.has_submodules",
        plan.target.has_submodules,
        fresh.target.has_submodules,
    )?;
    ensure_match(
        "target.operation",
        &format!("{:?}", plan.target_state.operation),
        &format!("{:?}", fresh.target.operation),
    )?;
    ensure_u32(
        "target.working_tree.staged",
        plan.target_state.working_tree.staged,
        fresh.target.working_tree.staged,
    )?;
    ensure_u32(
        "target.working_tree.unstaged",
        plan.target_state.working_tree.unstaged,
        fresh.target.working_tree.unstaged,
    )?;
    ensure_u32(
        "target.working_tree.untracked",
        plan.target_state.working_tree.untracked,
        fresh.target.working_tree.untracked,
    )?;
    ensure_u32(
        "target.working_tree.ignored",
        plan.target_state.working_tree.ignored,
        fresh.target.working_tree.ignored,
    )?;
    ensure_u32(
        "target.working_tree.conflict_count",
        plan.target_state.working_tree.conflict_count,
        fresh.target.working_tree.conflict_count,
    )?;

    Ok(())
}

fn trusted_git_args(target: &Path) -> Vec<OsString> {
    vec![
        OsString::from("worktree"),
        OsString::from("remove"),
        target.as_os_str().to_os_string(),
    ]
}

fn record_for(
    status: &str,
    plan: &WorktreeRemovePlan,
    trusted_git_args: Vec<String>,
) -> WorktreeRemoveExecutionRecord {
    WorktreeRemoveExecutionRecord {
        schema_version: WORKTREE_REMOVE_EXECUTION_RECORD_SCHEMA_VERSION.to_string(),
        status: status.to_string(),
        action: ACTION_WORKTREE_REMOVE.to_string(),
        plan_id: plan.plan_id.clone(),
        repository: WorktreeRemoveRepository {
            family_id: plan.repository.family_id.clone(),
            git_common_dir: plan.repository.git_common_dir.clone(),
            main_worktree: plan.repository.main_worktree.clone(),
            selected_from: plan.repository.selected_from.clone(),
        },
        target: WorktreeRemoveTarget {
            input_path: plan.target.input_path.clone(),
            canonical_path: plan.target.canonical_path.clone(),
            worktree_list_path: plan.target.worktree_list_path.clone(),
            kind: plan.target.kind.clone(),
            worktree_git_dir: plan.target.worktree_git_dir.clone(),
            git_common_dir: plan.target.git_common_dir.clone(),
            head: plan.target.head.clone(),
            branch: plan.target.branch.clone(),
            detached: plan.target.detached,
            locked: plan.target.locked,
            prunable: plan.target.prunable,
            is_current_worktree: plan.target.is_current_worktree,
            has_submodules: plan.target.has_submodules,
        },
        target_state: WorktreeRemoveTargetState {
            operation: plan.target_state.operation,
            working_tree: plan.target_state.working_tree.clone(),
        },
        confirmation_reason_codes: plan.confirmation.reason_codes.clone(),
        automatic_undo_available: false,
        undo_strategy: UnavailableUndoStrategy {
            kind: plan.undo_strategy.kind.clone(),
            reason: plan.undo_strategy.reason.clone(),
        },
        trusted_git_args,
    }
}

fn execution_record_path(plan: &WorktreeRemovePlan) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(b"super-git-worktree-remove-execute-v0.1\n");
    hasher.update(plan.plan_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(plan.target.worktree_list_path.to_string_lossy().as_bytes());
    let id = format_hex_digest(hasher.finalize().as_slice());
    plan.repository
        .git_common_dir
        .join("super-git")
        .join("executions")
        .join(format!("{id}.json"))
}

fn write_record_create_new(path: &Path, record: &WorktreeRemoveExecutionRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(record)?;
    let mut file = execute::create_new_or_already_attempted(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn write_record_replace(path: &Path, record: &WorktreeRemoveExecutionRecord) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(record)?;
    let tmp_path = path.with_extension("json.tmp");
    let write_result = (|| -> Result<()> {
        let mut tmp = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        tmp.write_all(&bytes)?;
        tmp.sync_all()?;
        drop(tmp);
        fs::rename(&tmp_path, path)?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    Ok(())
}

fn post_verify(plan: &WorktreeRemovePlan, repository: &Path) -> Result<()> {
    if plan.target.worktree_list_path.exists() {
        return mismatch("target_path_exists_after_remove", "false", "true");
    }
    let worktrees = worktree::list_worktrees(repository)?;
    if worktrees
        .iter()
        .any(|worktree| same_path(&worktree.path, &plan.target.worktree_list_path))
    {
        return mismatch("worktree_list.target", "absent", "present");
    }
    Ok(())
}

fn read_branch_oid(git: &Git, repo: &Path, branch: &str) -> Result<String> {
    let output = git.run_in(
        repo,
        ["rev-parse", "--verify", &format!("refs/heads/{branch}")],
    )?;
    Ok(output.stdout.trim().to_string())
}

fn partial_failure(
    error: impl std::fmt::Display,
    execution_record_path: &Path,
    repository: &Path,
    target_path: &Path,
) -> SuperGitError {
    SuperGitError::ExecutePartialFailure {
        action: ACTION_WORKTREE_REMOVE.to_string(),
        message: error.to_string(),
        execution_record_path: execution_record_path.to_path_buf(),
        target_path: target_path.to_path_buf(),
        target_path_exists: target_path.exists(),
        worktree_list_entry_present: worktree::list_worktrees(repository)
            .map(|worktrees| {
                worktrees
                    .iter()
                    .any(|worktree| same_path(&worktree.path, target_path))
            })
            .unwrap_or(false),
    }
}

fn ensure_match(field: &str, expected: &str, actual: &str) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        mismatch(field, expected, actual)
    }
}

fn ensure_option_match(
    field: &str,
    expected: &Option<String>,
    actual: &Option<String>,
) -> Result<()> {
    match (expected, actual) {
        (Some(expected), Some(actual)) => ensure_match(field, expected, actual),
        (None, None) => Ok(()),
        (Some(expected), None) => mismatch(field, expected, "null"),
        (None, Some(actual)) => mismatch(field, "null", actual),
    }
}

fn ensure_path_match(field: &str, expected: &Path, actual: &Path) -> Result<()> {
    ensure_match(
        field,
        &expected.display().to_string(),
        &actual.display().to_string(),
    )
}

fn ensure_path_option_match(
    field: &str,
    expected: &Option<PathBuf>,
    actual: &Option<PathBuf>,
) -> Result<()> {
    match (expected, actual) {
        (Some(expected), Some(actual)) => ensure_path_match(field, expected, actual),
        (None, None) => Ok(()),
        (Some(expected), None) => mismatch(field, &expected.display().to_string(), "null"),
        (None, Some(actual)) => mismatch(field, "null", &actual.display().to_string()),
    }
}

fn ensure_bool(field: &str, expected: bool, actual: bool) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        mismatch(field, &expected.to_string(), &actual.to_string())
    }
}

fn ensure_u32(field: &str, expected: u32, actual: u32) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        mismatch(field, &expected.to_string(), &actual.to_string())
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right
}

fn invalid_plan(code: &str, message: &str) -> Result<()> {
    Err(SuperGitError::ExecutePlanInvalid {
        code: code.to_string(),
        message: message.to_string(),
    })
}

fn mismatch(field: &str, expected: &str, actual: &str) -> Result<()> {
    Err(SuperGitError::ExecutePreconditionMismatch {
        field: field.to_string(),
        expected: expected.to_string(),
        actual: actual.to_string(),
    })
}

fn format_hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn display_arg(arg: &OsString) -> String {
    arg.to_string_lossy().into_owned()
}
