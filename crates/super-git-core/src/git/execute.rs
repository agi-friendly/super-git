use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::git::command::Git;
use crate::git::execute_worktree;
use crate::git::execute_worktree_remove;
use crate::git::fingerprint::{read_state_fingerprint, resolved_stage_changes_paths};
use crate::git::preview::compute_plan_id;
use crate::git::preview_worktree_remove;
use crate::git::state;
use crate::git::undo_registry;
use crate::model::{
    ExecuteResult, ExecuteUndoToken, Operation, PreviewPlan, PreviewPrecondition, UndoToken,
    WorktreeCreatePlan, WorktreeRemoveConfirmation, WorktreeRemovePlan,
    CONFIRMATION_SCHEMA_VERSION, DESTRUCTIVE_PREVIEW_PLAN_SCHEMA_VERSION, EXECUTE_SCHEMA_VERSION,
    PLAN_SCHEMA_VERSION, UNDO_TOKEN_SCHEMA_VERSION, WORKTREE_PLAN_SCHEMA_VERSION,
};
use crate::{Result, SuperGitError};

const ACTION_STAGE_CHANGES: &str = "stage_changes";
const ACTION_WORKTREE_REMOVE: &str = "worktree_remove";

pub fn execute_plan_bytes(current_path: &Path, bytes: &[u8]) -> Result<ExecuteResult> {
    execute_plan_bytes_with_confirmation(current_path, bytes, None)
}

pub fn execute_plan_bytes_with_confirmation(
    current_path: &Path,
    bytes: &[u8],
    confirmation_bytes: Option<&[u8]>,
) -> Result<ExecuteResult> {
    let plan = parse_plan(bytes)?;
    match plan {
        PlanToExecute::StageChanges(plan) => {
            reject_unexpected_confirmation(confirmation_bytes)?;
            execute_stage_changes_plan(current_path, *plan)
        }
        PlanToExecute::WorktreeCreate(plan) => {
            reject_unexpected_confirmation(confirmation_bytes)?;
            execute_worktree::execute_worktree_create_plan(*plan)
        }
        PlanToExecute::WorktreeRemove(plan) => {
            execute_worktree_remove_plan(current_path, *plan, confirmation_bytes)
        }
    }
}

enum PlanToExecute {
    StageChanges(Box<PreviewPlan>),
    WorktreeCreate(Box<WorktreeCreatePlan>),
    WorktreeRemove(Box<WorktreeRemovePlan>),
}

fn parse_plan(bytes: &[u8]) -> Result<PlanToExecute> {
    let value: Value = serde_json::from_slice(bytes)?;
    if value.get("ok").is_some() {
        if value.get("ok") != Some(&Value::Bool(true)) {
            return invalid_plan(
                "plan_envelope_not_success",
                "plan envelope must have ok=true",
            );
        }

        let data = value
            .get("data")
            .ok_or_else(|| SuperGitError::ExecutePlanInvalid {
                code: "missing_data".to_string(),
                message: "plan envelope must contain data".to_string(),
            })?;
        return parse_plan_value(data.clone());
    }

    parse_plan_value(value)
}

fn parse_plan_value(value: Value) -> Result<PlanToExecute> {
    match value.get("schema_version").and_then(Value::as_str) {
        Some(PLAN_SCHEMA_VERSION) => Ok(PlanToExecute::StageChanges(Box::new(
            serde_json::from_value(value)?,
        ))),
        Some(WORKTREE_PLAN_SCHEMA_VERSION) => Ok(PlanToExecute::WorktreeCreate(Box::new(
            serde_json::from_value(value)?,
        ))),
        Some(DESTRUCTIVE_PREVIEW_PLAN_SCHEMA_VERSION) => Ok(PlanToExecute::WorktreeRemove(
            Box::new(serde_json::from_value(value)?),
        )),
        _ => invalid_plan(
            "unsupported_schema_version",
            "execute supports only super-git.plan.v0.1, super-git.plan.v0.2, and super-git.plan.v0.3",
        ),
    }
}

fn execute_worktree_remove_plan(
    current_path: &Path,
    plan: WorktreeRemovePlan,
    confirmation_bytes: Option<&[u8]>,
) -> Result<ExecuteResult> {
    validate_worktree_remove_static_contract(&plan)?;
    let Some(confirmation_bytes) = confirmation_bytes else {
        return invalid_plan(
            "confirmation_required",
            "worktree_remove execute requires a separate confirmation artifact before delete support can be considered",
        );
    };

    let confirmation = parse_worktree_remove_confirmation(confirmation_bytes)?;
    validate_worktree_remove_confirmation(&plan, &confirmation)?;
    execute_worktree_remove::execute_worktree_remove_plan(current_path, plan)
}

fn validate_worktree_remove_static_contract(plan: &WorktreeRemovePlan) -> Result<()> {
    if plan.schema_version != DESTRUCTIVE_PREVIEW_PLAN_SCHEMA_VERSION {
        return invalid_plan(
            "unsupported_schema_version",
            "worktree_remove execute requires super-git.plan.v0.3",
        );
    }

    let expected_plan_id = preview_worktree_remove::compute_worktree_remove_plan_id(plan)?;
    ensure_match("plan_id", &plan.plan_id, &expected_plan_id)?;

    if plan.action.kind != ACTION_WORKTREE_REMOVE {
        return invalid_plan(
            "unsupported_action",
            "worktree_remove execute supports only worktree_remove plans",
        );
    }
    if !plan.confirmation.required_before_execute {
        return invalid_plan(
            "confirmation_required",
            "worktree_remove plans must require explicit confirmation before execute",
        );
    }
    if plan.undo_strategy.kind != "not_available" {
        return invalid_plan(
            "unsupported_undo_strategy",
            "worktree_remove execute cannot claim automatic undo support",
        );
    }

    Ok(())
}

fn reject_unexpected_confirmation(confirmation_bytes: Option<&[u8]>) -> Result<()> {
    if confirmation_bytes.is_some() {
        return invalid_plan(
            "confirmation_not_supported",
            "confirmation artifacts are supported only for destructive worktree_remove plans",
        );
    }
    Ok(())
}

fn parse_worktree_remove_confirmation(bytes: &[u8]) -> Result<WorktreeRemoveConfirmation> {
    let value: Value = serde_json::from_slice(bytes)?;
    if value.get("schema_version").and_then(Value::as_str) != Some(CONFIRMATION_SCHEMA_VERSION) {
        return invalid_plan(
            "confirmation_schema_unsupported",
            "worktree_remove confirmation requires super-git.confirmation.v0.1",
        );
    }

    serde_json::from_value(value).map_err(|err| SuperGitError::ExecutePlanInvalid {
        code: "confirmation_artifact_invalid".to_string(),
        message: format!("worktree_remove confirmation artifact is invalid JSON shape: {err}"),
    })
}

fn validate_worktree_remove_confirmation(
    plan: &WorktreeRemovePlan,
    confirmation: &WorktreeRemoveConfirmation,
) -> Result<()> {
    if confirmation.kind.as_deref() != Some("destructive_action_confirmation") {
        return invalid_plan(
            "confirmation_kind_unsupported",
            "worktree_remove confirmation kind must be destructive_action_confirmation",
        );
    }
    if confirmation.action.as_deref() != Some(plan.action.kind.as_str()) {
        return invalid_plan(
            "confirmation_action_mismatch",
            "worktree_remove confirmation action must match the plan action",
        );
    }
    if confirmation.plan_schema_version.as_deref() != Some(plan.schema_version.as_str())
        || confirmation.plan_id.as_deref() != Some(plan.plan_id.as_str())
    {
        return invalid_plan(
            "confirmation_plan_mismatch",
            "worktree_remove confirmation must match the plan schema version and plan id",
        );
    }

    validate_confirmation_target(plan, confirmation)?;

    if confirmation.acknowledged_reason_codes.as_ref() != Some(&plan.confirmation.reason_codes) {
        return invalid_plan(
            "confirmation_reason_codes_mismatch",
            "worktree_remove confirmation must acknowledge the exact plan reason codes",
        );
    }
    if confirmation.acknowledged_undo_strategy.as_deref() != Some(plan.undo_strategy.kind.as_str())
    {
        return invalid_plan(
            "confirmation_undo_strategy_mismatch",
            "worktree_remove confirmation must acknowledge the plan undo strategy",
        );
    }

    let Some(acknowledgement) = &confirmation.acknowledgement else {
        return invalid_plan(
            "confirmation_acknowledgement_missing",
            "worktree_remove confirmation must include an explicit acknowledgement",
        );
    };
    if acknowledgement.method.as_deref() != Some("cli_typed_phrase") {
        return invalid_plan(
            "confirmation_acknowledgement_missing",
            "worktree_remove confirmation acknowledgement method must be cli_typed_phrase",
        );
    }

    let expected_phrase = format!(
        "remove worktree {} without automatic undo",
        plan.target.worktree_list_path.display()
    );
    if acknowledgement.phrase.as_deref() != Some(expected_phrase.as_str()) {
        return invalid_plan(
            "confirmation_phrase_mismatch",
            "worktree_remove confirmation phrase must match the deterministic target phrase",
        );
    }

    Ok(())
}

fn validate_confirmation_target(
    plan: &WorktreeRemovePlan,
    confirmation: &WorktreeRemoveConfirmation,
) -> Result<()> {
    let Some(confirmation_target) = &confirmation.target else {
        return invalid_plan(
            "confirmation_target_mismatch",
            "worktree_remove confirmation target is required",
        );
    };
    let Some(plan_git_common_dir) = &plan.target.git_common_dir else {
        return invalid_plan(
            "confirmation_target_mismatch",
            "worktree_remove plan target must include git_common_dir",
        );
    };

    ensure_confirmation_path_match(
        "confirmation.target.worktree_list_path",
        &plan.target.worktree_list_path,
        &confirmation_target.worktree_list_path,
    )?;
    ensure_confirmation_path_match(
        "confirmation.target.git_common_dir",
        plan_git_common_dir,
        &confirmation_target.git_common_dir,
    )?;
    ensure_confirmation_optional_match(
        "confirmation.target.head",
        &plan.target.head,
        &confirmation_target.head,
    )?;
    ensure_confirmation_optional_match(
        "confirmation.target.branch",
        &plan.target.branch,
        &confirmation_target.branch,
    )
}

fn execute_stage_changes_plan(current_path: &Path, plan: PreviewPlan) -> Result<ExecuteResult> {
    validate_static_contract(&plan)?;

    let git = Git::default();
    let state = state::read_state(current_path)?;
    ensure_match(
        "repository",
        &plan.repository.display().to_string(),
        &state.root.display().to_string(),
    )?;

    validate_current_preconditions(&state)?;
    validate_current_fingerprint(&git, &state, &plan)?;
    validate_resolved_paths(&git, &state.root, &plan)?;

    let snapshot = snapshot_index(&git, &state.root, &plan)?;
    let mut args = vec![
        OsString::from("add"),
        OsString::from("--all"),
        OsString::from("--"),
    ];
    args.extend(plan.action.resolved_paths.iter().map(OsString::from));

    if let Err(err) = git.run_write_in(&state.root, args) {
        let _ = fs::remove_file(&snapshot.path);
        return Err(err);
    }

    let post_index_sha256 = hash_index(&snapshot.index_path)?;
    let undo_token = UndoToken {
        schema_version: UNDO_TOKEN_SCHEMA_VERSION.to_string(),
        kind: "restore_index_snapshot".to_string(),
        repository: state.root.clone(),
        action: ACTION_STAGE_CHANGES.to_string(),
        plan_id: plan.plan_id.clone(),
        target_paths: plan.action.resolved_paths.clone(),
        index_snapshot_path: snapshot.path.clone(),
        pre_index_existed: snapshot.pre_index_existed,
        pre_index_sha256: snapshot.pre_index_sha256.clone(),
        post_index_sha256,
    };
    if let Err(err) = undo_registry::write_record(&undo_token, &snapshot.undo_dir) {
        if let Err(rollback_err) = rollback_index_after_registry_failure(&snapshot) {
            return Err(SuperGitError::ExecuteRollbackFailed {
                original_error: err.to_string(),
                rollback_error: rollback_err.to_string(),
            });
        }
        return Err(err);
    }

    Ok(ExecuteResult {
        schema_version: EXECUTE_SCHEMA_VERSION.to_string(),
        plan_id: plan.plan_id,
        action: ACTION_STAGE_CHANGES.to_string(),
        repository: state.root,
        executed: true,
        effects: vec!["Staged previewed paths in the current worktree index.".to_string()],
        undo_token: Some(ExecuteUndoToken::Index(Box::new(undo_token))),
    })
}

fn validate_static_contract(plan: &PreviewPlan) -> Result<()> {
    if plan.schema_version != PLAN_SCHEMA_VERSION {
        return invalid_plan(
            "unsupported_schema_version",
            "execute supports only super-git.plan.v0.1",
        );
    }

    let expected_plan_id = compute_plan_id(plan)?;
    ensure_match("plan_id", &plan.plan_id, &expected_plan_id)?;

    if plan.action.kind != ACTION_STAGE_CHANGES {
        return invalid_plan("unsupported_action", "execute supports only stage_changes");
    }
    if plan.action.scope != "all" {
        return invalid_plan("unsupported_scope", "stage_changes supports only scope=all");
    }
    validate_pathset(&plan.action.resolved_paths)?;

    if plan.preconditions != expected_preconditions() {
        return invalid_plan(
            "unsupported_preconditions",
            "stage_changes plan preconditions do not match the supported execute contract",
        );
    }
    if plan.risk.severity != "low"
        || plan.risk.reversibility != "reversible"
        || plan.risk.requires_human_confirmation
    {
        return invalid_plan(
            "unsupported_risk",
            "stage_changes execute requires low reversible risk without human confirmation",
        );
    }
    if plan.undo_strategy.kind != "restore_index_snapshot"
        || !plan.undo_strategy.requires_index_snapshot
    {
        return invalid_plan(
            "unsupported_undo_strategy",
            "stage_changes execute requires restore_index_snapshot undo strategy",
        );
    }

    Ok(())
}

fn validate_current_preconditions(state: &crate::model::RepoState) -> Result<()> {
    if state.operation != Operation::None {
        return mismatch("operation_none", "true", "false");
    }
    if state.working_tree.conflict_count != 0 {
        return mismatch("no_conflicts", "true", "false");
    }
    if state.working_tree.staged != 0 {
        return mismatch("index_clean", "true", "false");
    }
    if state.working_tree.unstaged == 0 && state.working_tree.untracked == 0 {
        return mismatch("has_unstaged_or_untracked_changes", "true", "false");
    }
    Ok(())
}

fn validate_current_fingerprint(
    git: &Git,
    state: &crate::model::RepoState,
    plan: &PreviewPlan,
) -> Result<()> {
    let current = read_state_fingerprint(
        git,
        &state.root,
        &state.root,
        state.head.commit.clone(),
        state.operation,
    )?;

    if current == plan.state_fingerprint {
        return Ok(());
    }

    ensure_match(
        "state_fingerprint",
        &serde_json::to_string(&plan.state_fingerprint)?,
        &serde_json::to_string(&current)?,
    )
}

fn validate_resolved_paths(git: &Git, root: &Path, plan: &PreviewPlan) -> Result<()> {
    let current = resolved_stage_changes_paths(git, root)?;
    if current == plan.action.resolved_paths {
        return Ok(());
    }

    ensure_match(
        "action.resolved_paths",
        &serde_json::to_string(&plan.action.resolved_paths)?,
        &serde_json::to_string(&current)?,
    )
}

fn validate_pathset(paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return invalid_plan("empty_resolved_paths", "resolved_paths must not be empty");
    }

    let mut previous: Option<&str> = None;
    for path in paths {
        validate_relative_path(path)?;
        if let Some(previous) = previous {
            if previous >= path.as_str() {
                return invalid_plan(
                    "unstable_resolved_paths",
                    "resolved_paths must be sorted and unique",
                );
            }
        }
        previous = Some(path);
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<()> {
    if path.is_empty() || path.contains('\0') {
        return invalid_plan(
            "unsafe_resolved_path",
            "resolved path must be a non-empty repository-relative path",
        );
    }

    let mut components = Path::new(path).components();
    let Some(first) = components.next() else {
        return invalid_plan(
            "unsafe_resolved_path",
            "resolved path must contain at least one normal component",
        );
    };

    if !matches!(first, Component::Normal(_)) || first.as_os_str() == ".git" {
        return invalid_plan(
            "unsafe_resolved_path",
            "resolved path must stay inside the worktree and outside .git",
        );
    }
    for component in components {
        if !matches!(component, Component::Normal(_)) {
            return invalid_plan(
                "unsafe_resolved_path",
                "resolved path must not contain absolute, parent, or current-directory components",
            );
        }
    }

    Ok(())
}

struct IndexSnapshot {
    path: PathBuf,
    undo_dir: PathBuf,
    index_path: PathBuf,
    index_lock_path: PathBuf,
    pre_index_existed: bool,
    pre_index_sha256: String,
}

fn snapshot_index(git: &Git, root: &Path, plan: &PreviewPlan) -> Result<IndexSnapshot> {
    let index_path = git_path(git, root, "index")?;
    let index_lock_path = git_path(git, root, "index.lock")?;
    let index_state = read_index_state(&index_path)?;
    let pre_index_sha256 = sha256_hex(&index_state.bytes);
    let execution_id = execution_id(plan, &pre_index_sha256);
    let undo_dir = git_path(git, root, "super-git/undo")?;
    let snapshot_path = undo_dir.join(format!("{execution_id}.index"));

    if index_state.existed {
        fs::create_dir_all(&undo_dir)?;
        fs::write(&snapshot_path, &index_state.bytes)?;
    }

    Ok(IndexSnapshot {
        path: snapshot_path,
        undo_dir,
        index_path,
        index_lock_path,
        pre_index_existed: index_state.existed,
        pre_index_sha256,
    })
}

fn rollback_index_after_registry_failure(snapshot: &IndexSnapshot) -> Result<()> {
    let lock = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&snapshot.index_lock_path)?;

    if snapshot.pre_index_existed {
        let bytes = fs::read(&snapshot.path)?;
        restore_index_from_bytes(
            lock,
            &snapshot.index_path,
            &snapshot.index_lock_path,
            &bytes,
        )
    } else {
        remove_index_after_failed_execute(lock, &snapshot.index_path, &snapshot.index_lock_path)
    }
}

fn restore_index_from_bytes(
    mut lock: fs::File,
    index_path: &Path,
    lock_path: &Path,
    bytes: &[u8],
) -> Result<()> {
    lock.write_all(bytes)?;
    lock.sync_all()?;
    drop(lock);
    fs::rename(lock_path, index_path)?;
    Ok(())
}

fn remove_index_after_failed_execute(
    lock: fs::File,
    index_path: &Path,
    lock_path: &Path,
) -> Result<()> {
    drop(lock);
    match fs::remove_file(index_path) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => {
            let _ = fs::remove_file(lock_path);
            return Err(err.into());
        }
    }
    fs::remove_file(lock_path)?;
    Ok(())
}

fn git_path(git: &Git, root: &Path, path: &str) -> Result<PathBuf> {
    let output = git.run_in(
        root,
        ["rev-parse", "--path-format=absolute", "--git-path", path],
    )?;
    Ok(PathBuf::from(output.stdout.trim()))
}

fn hash_index(path: &Path) -> Result<String> {
    Ok(sha256_hex(&read_index_state(path)?.bytes))
}

struct IndexState {
    existed: bool,
    bytes: Vec<u8>,
}

fn read_index_state(path: &Path) -> Result<IndexState> {
    match fs::read(path) {
        Ok(bytes) => Ok(IndexState {
            existed: true,
            bytes,
        }),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(IndexState {
            existed: false,
            bytes: Vec::new(),
        }),
        Err(err) => Err(err.into()),
    }
}

fn execution_id(plan: &PreviewPlan, pre_index_sha256: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"super-git-execute-v0.1\n");
    hasher.update(plan.plan_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(pre_index_sha256.as_bytes());
    format_hex_digest(hasher.finalize().as_slice())
}

fn expected_preconditions() -> Vec<PreviewPrecondition> {
    vec![
        passed("operation_none"),
        passed("no_conflicts"),
        passed("index_clean"),
        passed("has_unstaged_or_untracked_changes"),
    ]
}

fn passed(code: &str) -> PreviewPrecondition {
    PreviewPrecondition {
        code: code.to_string(),
        status: "passed".to_string(),
    }
}

fn invalid_plan<T>(code: &str, message: &str) -> Result<T> {
    Err(SuperGitError::ExecutePlanInvalid {
        code: code.to_string(),
        message: message.to_string(),
    })
}

fn ensure_match(field: &str, expected: &str, actual: &str) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    mismatch(field, expected, actual)
}

fn ensure_confirmation_path_match(
    field: &str,
    expected: &Path,
    actual: &Option<PathBuf>,
) -> Result<()> {
    let Some(actual) = actual else {
        return invalid_plan(
            "confirmation_target_mismatch",
            &format!("{field} is required"),
        );
    };
    if expected == actual {
        return Ok(());
    }
    invalid_plan(
        "confirmation_target_mismatch",
        &format!(
            "{field} expected {} but confirmation has {}",
            expected.display(),
            actual.display()
        ),
    )
}

fn ensure_confirmation_optional_match(
    field: &str,
    expected: &Option<String>,
    actual: &Option<String>,
) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    invalid_plan(
        "confirmation_target_mismatch",
        &format!("{field} expected {expected:?} but confirmation has {actual:?}"),
    )
}

fn mismatch<T>(field: &str, expected: &str, actual: &str) -> Result<T> {
    Err(SuperGitError::ExecutePreconditionMismatch {
        field: field.to_string(),
        expected: expected.to_string(),
        actual: actual.to_string(),
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format_digest(hasher.finalize().as_slice())
}

fn format_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity("sha256:".len() + bytes.len() * 2);
    output.push_str("sha256:");
    output.push_str(&format_hex_digest(bytes));
    output
}

fn format_hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(hex_char(byte >> 4));
        output.push(hex_char(byte & 0x0f));
    }
    output
}

fn hex_char(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + value - 10) as char,
        _ => unreachable!("nibble is always <= 15"),
    }
}
