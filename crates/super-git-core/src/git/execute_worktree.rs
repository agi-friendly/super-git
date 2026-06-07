use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::config::store::SavedRepository;
use crate::git::command::Git;
use crate::git::{preview_worktree, worktree};
use crate::model::{
    ExecuteResult, ExecuteUndoToken, WorktreeCreatePlan, WorktreeExecutionRecord, WorktreeInfo,
    WorktreeUndoToken, EXECUTE_SCHEMA_VERSION, UNDO_TOKEN_SCHEMA_VERSION,
    WORKTREE_EXECUTION_RECORD_SCHEMA_VERSION, WORKTREE_PLAN_SCHEMA_VERSION,
};
use crate::{Result, SuperGitError};

const ACTION_WORKTREE_CREATE: &str = "worktree_create";
const UNDO_REMOVE_CREATED_WORKTREE: &str = "remove_created_worktree_if_clean";

pub fn execute_worktree_create_plan(plan: WorktreeCreatePlan) -> Result<ExecuteResult> {
    validate_static_contract(&plan)?;

    let git = Git::default();
    validate_repository_identity(&plan)?;
    validate_source_ref(&git, &plan)?;
    let worktrees = worktree::list_worktrees(&plan.repository.selected_from)?;
    validate_family_snapshot(&worktrees, &plan)?;
    validate_target(&plan, &worktrees)?;

    let execution_record_path = execution_record_path(&plan);
    let expected_head = plan
        .source_ref
        .resolved_commit
        .clone()
        .expect("static validation requires resolved commit");
    let expected_branch = expected_branch(&plan);
    let created_parent = plan
        .target
        .parent_creation
        .will_create
        .then(|| plan.target.parent.clone());
    let intent = WorktreeExecutionRecord {
        schema_version: WORKTREE_EXECUTION_RECORD_SCHEMA_VERSION.to_string(),
        status: "intent".to_string(),
        action: ACTION_WORKTREE_CREATE.to_string(),
        plan_id: plan.plan_id.clone(),
        repository: plan.repository.clone(),
        target_path: plan.target.path.clone(),
        source_ref: plan.source_ref.clone(),
        expected_head: expected_head.clone(),
        expected_branch: expected_branch.clone(),
        created_parent: created_parent.clone(),
        undo_token: None,
    };
    write_record_create_new(&execution_record_path, &intent)?;

    if plan.target.parent_creation.will_create {
        fs::create_dir(&plan.target.parent).map_err(|err| {
            partial_failure(
                err,
                &execution_record_path,
                &plan.repository.selected_from,
                &plan.target.path,
            )
        })?;
    }

    if let Err(err) = git.run_write_in(&plan.repository.selected_from, trusted_git_args(&plan)?) {
        return Err(partial_failure(
            err,
            &execution_record_path,
            &plan.repository.selected_from,
            &plan.target.path,
        ));
    }

    post_verify(&git, &plan, &expected_head, expected_branch.as_deref()).map_err(|err| {
        partial_failure(
            err,
            &execution_record_path,
            &plan.repository.selected_from,
            &plan.target.path,
        )
    })?;

    let undo_token = WorktreeUndoToken {
        schema_version: UNDO_TOKEN_SCHEMA_VERSION.to_string(),
        kind: UNDO_REMOVE_CREATED_WORKTREE.to_string(),
        repository: plan.repository.selected_from.clone(),
        action: ACTION_WORKTREE_CREATE.to_string(),
        plan_id: plan.plan_id.clone(),
        target_path: plan.target.path.clone(),
        target_head: expected_head,
        target_branch: expected_branch,
        git_common_dir: plan.repository.git_common_dir.clone(),
        family_id: plan.repository.family_id.clone(),
        source_ref: plan.source_ref.clone(),
        ref_policy: plan.ref_policy.clone(),
        created_parent,
        execution_record_path: execution_record_path.clone(),
        deletes_branch: false,
        deletes_history: false,
    };
    let completed = WorktreeExecutionRecord {
        status: "completed".to_string(),
        undo_token: Some(undo_token.clone()),
        ..intent
    };
    write_record_replace(&execution_record_path, &completed).map_err(|err| {
        partial_failure(
            err,
            &execution_record_path,
            &plan.repository.selected_from,
            &plan.target.path,
        )
    })?;

    Ok(ExecuteResult {
        schema_version: EXECUTE_SCHEMA_VERSION.to_string(),
        plan_id: plan.plan_id,
        action: ACTION_WORKTREE_CREATE.to_string(),
        repository: plan.repository.selected_from,
        executed: true,
        effects: vec![format!(
            "Created linked worktree at {}.",
            plan.target.path.display()
        )],
        undo_token: Some(ExecuteUndoToken::Worktree(Box::new(undo_token))),
    })
}

fn validate_static_contract(plan: &WorktreeCreatePlan) -> Result<()> {
    if plan.schema_version != WORKTREE_PLAN_SCHEMA_VERSION {
        return invalid_plan(
            "unsupported_schema_version",
            "worktree_create execute supports only super-git.plan.v0.2",
        );
    }

    let expected_plan_id = preview_worktree::compute_worktree_plan_id(plan)?;
    ensure_match("plan_id", &plan.plan_id, &expected_plan_id)?;

    if plan.action.kind != ACTION_WORKTREE_CREATE {
        return invalid_plan(
            "unsupported_action",
            "worktree_create execute supports only worktree_create",
        );
    }
    if plan.execution.status != "executable" {
        return invalid_plan(
            "not_executable",
            "worktree_create execute requires execution.status=executable",
        );
    }
    if !plan.execution.super_git_execute_required
        || plan.execution.raw_git_allowed
        || !plan.execution.blocked_reasons.is_empty()
    {
        return invalid_plan(
            "unsupported_execution_contract",
            "worktree_create execute requires super-git-only executable plans without blocked reasons",
        );
    }
    if !plan.source_ref.supported_for_execute {
        return invalid_plan(
            "unsupported_source_ref",
            "worktree_create execute requires a supported local branch, tag, or commit ref",
        );
    }
    if plan.source_ref.resolved_commit.is_none() {
        return invalid_plan(
            "missing_resolved_commit",
            "worktree_create execute requires source_ref.resolved_commit",
        );
    }
    if !matches!(
        plan.source_ref.kind.as_str(),
        "local_branch" | "tag" | "commit"
    ) {
        return invalid_plan(
            "unsupported_source_ref_kind",
            "worktree_create execute supports local_branch, tag, and commit refs",
        );
    }
    validate_ref_policy_consistency(plan)?;
    if plan.ref_policy.will_create_branch || plan.ref_policy.will_track_upstream {
        return invalid_plan(
            "unsupported_ref_policy",
            "worktree_create execute does not create branches or tracking relationships",
        );
    }
    if plan.risk.severity != "medium"
        || plan.risk.reversibility != "reversible_if_unchanged"
        || plan.risk.requires_human_confirmation
    {
        return invalid_plan(
            "unsupported_risk",
            "worktree_create execute requires medium reversible_if_unchanged risk without human confirmation",
        );
    }
    if plan.undo_strategy.kind != UNDO_REMOVE_CREATED_WORKTREE
        || plan.undo_strategy.deletes_branch
        || plan.undo_strategy.deletes_history
    {
        return invalid_plan(
            "unsupported_undo_strategy",
            "worktree_create execute undo must not delete branches or history",
        );
    }
    if plan
        .preconditions
        .iter()
        .any(|precondition| precondition.status != "passed")
    {
        return invalid_plan(
            "preconditions_not_passed",
            "worktree_create execute requires all preconditions to be passed",
        );
    }
    validate_static_target(plan)?;
    Ok(())
}

fn validate_ref_policy_consistency(plan: &WorktreeCreatePlan) -> Result<()> {
    match plan.source_ref.kind.as_str() {
        "local_branch" => {
            ensure_ref_policy(
                plan,
                "existing_local_branch",
                false,
                "local branches must use existing_local_branch without detach",
            )?;
            ensure_full_ref(
                plan,
                &format!("refs/heads/{}", plan.source_ref.input),
                "local branches must carry refs/heads/<input> as source_ref.full_ref",
            )
        }
        "tag" => {
            ensure_ref_policy(
                plan,
                "detached_head",
                true,
                "tags must use detached_head with detach",
            )?;
            ensure_full_ref(
                plan,
                &format!("refs/tags/{}", plan.source_ref.input),
                "tags must carry refs/tags/<input> as source_ref.full_ref",
            )
        }
        "commit" => {
            ensure_ref_policy(
                plan,
                "detached_head",
                true,
                "commits must use detached_head with detach",
            )?;
            if plan.source_ref.full_ref.is_some() {
                return invalid_plan(
                    "ref_policy_consistency",
                    "commit refs must not carry source_ref.full_ref",
                );
            }
            Ok(())
        }
        _ => invalid_plan(
            "unsupported_source_ref_kind",
            "worktree_create execute supports local_branch, tag, and commit refs",
        ),
    }
}

fn ensure_full_ref(plan: &WorktreeCreatePlan, expected: &str, message: &str) -> Result<()> {
    if plan.source_ref.full_ref.as_deref() != Some(expected) {
        return invalid_plan("ref_policy_consistency", message);
    }
    Ok(())
}

fn ensure_ref_policy(
    plan: &WorktreeCreatePlan,
    expected_mode: &str,
    expected_detach: bool,
    message: &str,
) -> Result<()> {
    if plan.ref_policy.mode != expected_mode || plan.ref_policy.will_detach_head != expected_detach
    {
        return invalid_plan("ref_policy_consistency", message);
    }
    Ok(())
}

fn validate_static_target(plan: &WorktreeCreatePlan) -> Result<()> {
    for (field, path) in [
        ("target.path", &plan.target.path),
        ("target.parent", &plan.target.parent),
        ("repository.git_common_dir", &plan.repository.git_common_dir),
        ("repository.selected_from", &plan.repository.selected_from),
    ] {
        validate_absolute_clean_path(field, path)?;
    }

    if plan.target.path.parent() != Some(plan.target.parent.as_path()) {
        return invalid_plan(
            "target_parent_mismatch",
            "target.path must be directly under target.parent",
        );
    }
    if plan.target.path.file_name().and_then(|name| name.to_str()) != Some(&plan.target.name) {
        return invalid_plan(
            "target_name_mismatch",
            "target.path file name must match target.name",
        );
    }
    if plan.target.exists
        || plan.target.inside_git_dir
        || plan.target.inside_existing_worktree
        || plan.target.case_insensitive_collision
        || plan.target.reserved_name_collision
        || !plan.target.parent_creation.allowed
    {
        return invalid_plan(
            "unsafe_target_contract",
            "worktree_create execute requires an unblocked target contract",
        );
    }
    Ok(())
}

fn validate_absolute_clean_path(field: &str, path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return invalid_plan("unsafe_path", &format!("{field} must be an absolute path"));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return invalid_plan(
            "unsafe_path",
            &format!("{field} must not contain parent/current directory components"),
        );
    }
    Ok(())
}

fn validate_repository_identity(plan: &WorktreeCreatePlan) -> Result<()> {
    let current = SavedRepository::from_path(&plan.repository.selected_from)?;
    ensure_match(
        "repository.family_id",
        &plan.repository.family_id,
        &current.id,
    )?;
    ensure_path_match(
        "repository.git_common_dir",
        &plan.repository.git_common_dir,
        &current.git_common_dir,
    )?;
    ensure_optional_path_match(
        "repository.main_worktree",
        plan.repository.main_worktree.as_deref(),
        current.main_worktree.as_deref(),
    )?;
    ensure_path_match(
        "repository.selected_from",
        &plan.repository.selected_from,
        &current.saved_from,
    )
}

fn validate_source_ref(git: &Git, plan: &WorktreeCreatePlan) -> Result<()> {
    let refspec = match plan.source_ref.kind.as_str() {
        "local_branch" | "tag" => plan
            .source_ref
            .full_ref
            .as_deref()
            .ok_or_else(|| SuperGitError::ExecutePlanInvalid {
                code: "missing_full_ref".to_string(),
                message: "local branch and tag refs require source_ref.full_ref".to_string(),
            })?
            .to_string(),
        "commit" => plan.source_ref.input.clone(),
        _ => {
            return invalid_plan(
                "unsupported_source_ref_kind",
                "worktree_create execute supports local_branch, tag, and commit refs",
            );
        }
    };
    let output = git.run_in(
        &plan.repository.selected_from,
        ["rev-parse", "--verify", &format!("{refspec}^{{commit}}")],
    )?;
    let current = output.stdout.trim();
    let expected = plan
        .source_ref
        .resolved_commit
        .as_deref()
        .expect("static validation requires resolved commit");
    ensure_match("source_ref.resolved_commit", expected, current)
}

fn validate_family_snapshot(worktrees: &[WorktreeInfo], plan: &WorktreeCreatePlan) -> Result<()> {
    let current = preview_worktree::family_snapshot(worktrees)?;
    ensure_match(
        "family_snapshot.fingerprint",
        &plan.family_snapshot.fingerprint,
        &current.fingerprint,
    )
}

fn validate_target(plan: &WorktreeCreatePlan, worktrees: &[WorktreeInfo]) -> Result<()> {
    if plan.target.path.exists() {
        return mismatch("target_path_available", "true", "false");
    }
    ensure_bool(
        "target.parent_exists",
        plan.target.parent_exists,
        plan.target.parent.exists(),
    )?;
    ensure_bool(
        "target.parent_is_directory",
        plan.target.parent_is_directory,
        plan.target.parent.is_dir(),
    )?;
    ensure_bool(
        "target.parent_is_symlink",
        plan.target.parent_is_symlink,
        is_symlink(&plan.target.parent),
    )?;
    ensure_bool(
        "target.inside_git_dir",
        plan.target.inside_git_dir,
        is_inside(&plan.target.path, &plan.repository.git_common_dir),
    )?;
    ensure_bool(
        "target.inside_existing_worktree",
        plan.target.inside_existing_worktree,
        worktrees
            .iter()
            .filter(|worktree| !worktree.bare)
            .any(|worktree| is_inside(&plan.target.path, &worktree.path)),
    )?;
    ensure_bool(
        "target.case_insensitive_collision",
        plan.target.case_insensitive_collision,
        has_case_insensitive_collision(&plan.target.name, &plan.target.parent, worktrees),
    )?;
    ensure_bool(
        "target.reserved_name_collision",
        plan.target.reserved_name_collision,
        is_windows_reserved_component(&plan.target.name),
    )?;
    if !plan.target.parent_creation.allowed {
        return mismatch("target.parent_creation.allowed", "true", "false");
    }
    if plan.target.parent_creation.will_create && plan.target.parent.exists() {
        return mismatch("target.parent_exists", "false", "true");
    }
    if !plan.target.parent_creation.will_create && !plan.target.parent.is_dir() {
        return mismatch("target.parent_is_directory", "true", "false");
    }
    Ok(())
}

fn trusted_git_args(plan: &WorktreeCreatePlan) -> Result<Vec<OsString>> {
    let mut args = vec![
        OsString::from("worktree"),
        OsString::from("add"),
        OsString::from("-q"),
    ];
    if matches!(plan.source_ref.kind.as_str(), "tag" | "commit") {
        args.push(OsString::from("--detach"));
    }
    args.push(plan.target.path.as_os_str().to_os_string());
    args.push(OsString::from(checkout_arg(plan)?));
    Ok(args)
}

fn checkout_arg(plan: &WorktreeCreatePlan) -> Result<String> {
    match plan.source_ref.kind.as_str() {
        "local_branch" => Ok(plan
            .source_ref
            .full_ref
            .as_deref()
            .and_then(|full_ref| full_ref.strip_prefix("refs/heads/"))
            .unwrap_or(&plan.source_ref.input)
            .to_string()),
        "tag" => Ok(plan
            .source_ref
            .full_ref
            .clone()
            .unwrap_or_else(|| plan.source_ref.input.clone())),
        "commit" => Ok(plan
            .source_ref
            .resolved_commit
            .clone()
            .expect("static validation requires resolved commit")),
        _ => invalid_plan(
            "unsupported_source_ref_kind",
            "worktree_create execute supports local_branch, tag, and commit refs",
        ),
    }
}

fn post_verify(
    git: &Git,
    plan: &WorktreeCreatePlan,
    expected_head: &str,
    expected_branch: Option<&str>,
) -> Result<()> {
    if !plan.target.path.is_dir() {
        return mismatch("target.path", "directory", "missing");
    }
    let worktrees = worktree::list_worktrees(&plan.repository.selected_from)?;
    if !worktrees
        .iter()
        .any(|worktree| same_path(&worktree.path, &plan.target.path))
    {
        return mismatch("worktree_list.target", "present", "absent");
    }
    let current_head = git
        .run_in(
            &plan.target.path,
            ["rev-parse", "--verify", "HEAD^{commit}"],
        )?
        .stdout
        .trim()
        .to_string();
    ensure_match("target.head", expected_head, &current_head)?;

    let branch_result = git.try_run_in(&plan.target.path, ["symbolic-ref", "-q", "HEAD"])?;
    let current_branch = branch_result
        .success
        .then(|| branch_result.stdout.trim().to_string());
    match (expected_branch, current_branch.as_deref()) {
        (Some(expected), Some(actual)) => ensure_match("target.branch", expected, actual),
        (None, None) => Ok(()),
        (Some(expected), None) => mismatch("target.branch", expected, "detached"),
        (None, Some(actual)) => mismatch("target.branch", "detached", actual),
    }
}

fn expected_branch(plan: &WorktreeCreatePlan) -> Option<String> {
    (plan.source_ref.kind == "local_branch")
        .then(|| plan.source_ref.full_ref.clone())
        .flatten()
}

fn execution_record_path(plan: &WorktreeCreatePlan) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(b"super-git-worktree-execute-v0.1\n");
    hasher.update(plan.plan_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(plan.target.path.to_string_lossy().as_bytes());
    let id = format_hex_digest(hasher.finalize().as_slice());
    plan.repository
        .git_common_dir
        .join("super-git")
        .join("executions")
        .join(format!("{id}.json"))
}

fn write_record_create_new(path: &Path, record: &WorktreeExecutionRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(record)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn write_record_replace(path: &Path, record: &WorktreeExecutionRecord) -> Result<()> {
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

fn partial_failure(
    error: impl std::fmt::Display,
    execution_record_path: &Path,
    repository: &Path,
    target_path: &Path,
) -> SuperGitError {
    SuperGitError::ExecutePartialFailure {
        action: ACTION_WORKTREE_CREATE.to_string(),
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

fn ensure_bool(field: &str, expected: bool, actual: bool) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    mismatch(field, &expected.to_string(), &actual.to_string())
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

fn ensure_match(field: &str, expected: &str, actual: &str) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    mismatch(field, expected, actual)
}

fn mismatch<T>(field: &str, expected: &str, actual: &str) -> Result<T> {
    Err(SuperGitError::ExecutePreconditionMismatch {
        field: field.to_string(),
        expected: expected.to_string(),
        actual: actual.to_string(),
    })
}

fn invalid_plan<T>(code: &str, message: &str) -> Result<T> {
    Err(SuperGitError::ExecutePlanInvalid {
        code: code.to_string(),
        message: message.to_string(),
    })
}

fn is_symlink(path: &Path) -> bool {
    path.symlink_metadata()
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn is_inside(path: &Path, parent: &Path) -> bool {
    path == parent || path.starts_with(parent)
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || std::fs::canonicalize(left)
            .ok()
            .zip(std::fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
}

fn has_case_insensitive_collision(
    target_name: &str,
    parent: &Path,
    existing_worktrees: &[WorktreeInfo],
) -> bool {
    let target = target_name.to_lowercase();

    if existing_worktrees.iter().any(|worktree| {
        worktree
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_lowercase() == target)
    }) {
        return true;
    }

    let Ok(entries) = fs::read_dir(parent) else {
        return false;
    };

    entries.filter_map(std::result::Result::ok).any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.to_lowercase() == target)
    })
}

fn is_windows_reserved_component(value: &str) -> bool {
    let basename = value.split('.').next().unwrap_or(value);
    let uppercase = basename.to_ascii_uppercase();

    matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || is_numbered_reserved(&uppercase, "COM")
        || is_numbered_reserved(&uppercase, "LPT")
}

fn is_numbered_reserved(value: &str, prefix: &str) -> bool {
    let Some(suffix) = value.strip_prefix(prefix) else {
        return false;
    };

    suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
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
