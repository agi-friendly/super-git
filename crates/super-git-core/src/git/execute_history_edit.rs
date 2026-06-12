use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use serde_json::Value;

use crate::git::command::Git;
use crate::git::execute;
use crate::git::preview_history_edit;
use crate::git::preview_history_edit::{compute_history_edit_plan_id, preview_history_edit};
use crate::model::{
    ExecuteResult, ExecuteUndoToken, HistoryEditConfirmation, HistoryEditExecutionRecord,
    HistoryEditPlan, HistoryEditPlanCommit, HistoryEditPrediction, HistoryEditPredictionStep,
    HistoryEditUndoToken, CONFIRMATION_SCHEMA_VERSION, EXECUTE_SCHEMA_VERSION,
    HISTORY_EDIT_EXECUTION_RECORD_SCHEMA_VERSION, HISTORY_EDIT_PLAN_SCHEMA_VERSION,
    HISTORY_EDIT_UNDO_TOKEN_SCHEMA_VERSION,
};
use crate::{Result, SuperGitError};

const ACTION_HISTORY_EDIT: &str = "history_edit";

pub fn execute_history_edit_plan(
    current_path: &Path,
    plan: HistoryEditPlan,
    confirmation_bytes: Option<&[u8]>,
) -> Result<ExecuteResult> {
    validate_static_contract(&plan)?;
    // Published rewrites (preview_only) require a separate confirmation
    // artifact, validated statically against the plan before any write.
    // Unpublished (executable) plans must not carry one.
    match plan.execution.status.as_str() {
        "preview_only" => {
            let Some(bytes) = confirmation_bytes else {
                return invalid_plan(
                    "confirmation_required",
                    "history_edit execute for a published range requires a separate confirmation artifact",
                );
            };
            let confirmation = parse_confirmation(bytes)?;
            validate_confirmation(&plan, &confirmation)?;
        }
        _ => {
            if confirmation_bytes.is_some() {
                return invalid_plan(
                    "confirmation_not_supported",
                    "unpublished history_edit plans do not take a confirmation artifact",
                );
            }
        }
    }

    // The whole write path runs off `fresh`, not the caller-supplied `plan`.
    // Author identity and picked-commit messages are excluded from plan_id (the
    // contract treats them as derivable from the bound object ids), so trusting
    // the plan's copy would let a tampered plan forge them while passing every
    // check. `fresh` re-reads those fields from the live repository, and its
    // bound fields are proven identical to the plan by the plan_id match.
    let fresh = validate_fresh_binding(current_path, &plan)?;

    let git = Git::default();
    let worktree_root = fresh.repository.worktree_root.clone();
    let branch = fresh
        .branch
        .as_ref()
        .expect("executable plan has a branch (validated above)");
    let branch_ref = branch.ref_name.clone();
    let previous_tip = branch.tip_commit.clone();

    // drop 분류는 fresh 기준이다(static contract가 plan과의 동치를 검증했고,
    // plan_id 일치가 둘이 같음을 증명한다).
    let drop_prediction = fresh.prediction.clone();

    // drop: ref 이동 후의 read-tree 동기화가 사용자 변경을 덮어쓰지 않도록,
    // 어떤 write보다 먼저 clean 워킹트리를 하드 요구한다. untracked도 dirty다 —
    // 드랍된 delete는 sync에서 경로를 부활시키며 그 자리의 untracked 파일을
    // 덮어쓸 수 있다.
    if drop_prediction.is_some() {
        ensure_clean_working_tree(&git, &worktree_root)?;
    }

    // The live tip is the compare-and-swap old value. The fresh binding already
    // proved it matches the plan, but re-reading closes the tiny window between
    // revalidation and the ref move.
    let live_tip = read_ref_oid(&git, &worktree_root, &branch_ref)?;
    if live_tip != previous_tip {
        return mismatch("branch.tip_commit", &previous_tip, &live_tip);
    }
    let old_tree = read_tree_oid(&git, &worktree_root, &previous_tip)?;
    // post-verify 오라클: drop은 fresh 예측의 final_tree, tree-preserving은
    // 원래 tip의 tree(트리 보존 불변식).
    let expected_tree = match &drop_prediction {
        Some(prediction) => prediction
            .final_tree
            .clone()
            .expect("validated drop plan carries a final_tree"),
        None => old_tree.clone(),
    };
    // Status reads are best-effort: a failure reading status after the ref has
    // moved must not turn a completed, verified rewrite into a reported failure.
    // drop은 sync가 워킹트리를 바꾸는 것이 계약이라 드리프트 비교가 무의미하다.
    let status_before = if drop_prediction.is_none() {
        read_status_signature(&git, &worktree_root).ok()
    } else {
        None
    };

    let (groups, groups_before_first_drop) = build_groups(&fresh)?;
    let new_tip = match &drop_prediction {
        Some(prediction) => rebuild_replayed_commits(
            &git,
            &worktree_root,
            &fresh.range.base_commit,
            &groups,
            &prediction.steps,
            groups_before_first_drop,
        )?,
        None => rebuild_commits(&git, &worktree_root, &fresh.range.base_commit, &groups)?,
    };
    if new_tip == previous_tip {
        // The supported op set always changes at least one commit, so an
        // unchanged tip means the rebuild logic is wrong; never move the ref.
        return mismatch("rebuilt_tip", "different_from_previous_tip", &new_tip);
    }
    // ref가 움직이기 전의 마지막 안전망: 재구축된 tip의 실제 tree가 오라클과
    // 일치해야 한다. 불일치는 구현 버그이고, 아무것도 움직이지 않은 채 멈춘다.
    let new_tree = read_tree_oid(&git, &worktree_root, &new_tip)?;
    if new_tree != expected_tree {
        return mismatch("pre_move_verify.final_tree", &expected_tree, &new_tree);
    }
    // 새 tip이 부활시키는 tracked 경로 위의 ignored 파일은 sync가 덮어쓴다 —
    // new_tip이 정해진 지금, ref가 움직이기 전에 막는다.
    if drop_prediction.is_some() {
        ensure_no_ignored_path_collisions(&git, &worktree_root, &new_tip)?;
    }

    let record_path = execution_record_path(&fresh, &branch_ref);
    let commits_after = groups.len() as u32;
    let intent = HistoryEditExecutionRecord {
        schema_version: HISTORY_EDIT_EXECUTION_RECORD_SCHEMA_VERSION.to_string(),
        status: "intent".to_string(),
        action: ACTION_HISTORY_EDIT.to_string(),
        plan_id: fresh.plan_id.clone(),
        repository: fresh.repository.clone(),
        branch_ref: branch_ref.clone(),
        previous_tip: previous_tip.clone(),
        new_tip: new_tip.clone(),
        final_tree: expected_tree.clone(),
        commits_before: fresh.range.commit_count as u32,
        commits_after,
        undo_token: None,
    };
    write_record_create_new(&record_path, &intent)?;

    // Compare-and-swap: only move the branch if it still points at previous_tip.
    // A lost race fails cleanly with no state change, so the intent record is
    // removed to avoid an orphan.
    if let Err(err) =
        compare_and_swap_ref(&git, &worktree_root, &branch_ref, &new_tip, &previous_tip)
    {
        let _ = fs::remove_file(&record_path);
        return Err(err);
    }

    if let Err(err) = post_verify(
        &git,
        &worktree_root,
        &fresh,
        &new_tip,
        &expected_tree,
        commits_after,
    ) {
        // sync는 아직 시작 전이므로 워킹트리는 미접촉이고, ref 롤백이
        // 정직하게 pre-execute 상태를 복원한다(drop 포함).
        return Err(rollback(
            &git,
            &worktree_root,
            &branch_ref,
            &previous_tip,
            &new_tip,
            &record_path,
            err,
        ));
    }

    // drop: index/워킹트리를 새 tip으로 동기화한다(rebase 의미론). ref는 이미
    // 올바른 새 tip이므로 여기서부터의 실패는 rollback이 아니라 partial
    // failure다 — record가 intent로 남아 undo와 재실행 모두 fail-closed가 된다.
    if drop_prediction.is_some() {
        if let Err(err) = sync_working_tree(&git, &worktree_root, &new_tip) {
            return Err(sync_partial_failure(
                &git,
                &worktree_root,
                &branch_ref,
                &record_path,
                false,
                err,
            ));
        }
    }

    let status_after = if drop_prediction.is_none() {
        read_status_signature(&git, &worktree_root).ok()
    } else {
        None
    };
    let mut effects = success_effects(&fresh, &branch_ref, &new_tip);
    // The drift note only fires when both reads succeed and disagree.
    if let (Some(before), Some(after)) = (&status_before, &status_after) {
        if before != after {
            effects.push(
                "Note: working-tree status changed during execute (external edit); the branch edit still applied.".to_string(),
            );
        }
    }

    let undo_token = HistoryEditUndoToken {
        schema_version: HISTORY_EDIT_UNDO_TOKEN_SCHEMA_VERSION.to_string(),
        kind: if drop_prediction.is_some() {
            "restore_branch_tip_and_worktree"
        } else {
            "restore_branch_tip_snapshot"
        }
        .to_string(),
        repository: worktree_root.clone(),
        action: ACTION_HISTORY_EDIT.to_string(),
        plan_id: fresh.plan_id.clone(),
        branch_ref: branch_ref.clone(),
        previous_tip: previous_tip.clone(),
        new_tip: new_tip.clone(),
        git_common_dir: fresh.repository.git_common_dir.clone(),
        family_id: fresh.repository.family_id.clone(),
        execution_record_path: record_path.clone(),
        deletes_branch: false,
        deletes_history: false,
    };
    let completed = HistoryEditExecutionRecord {
        status: "completed".to_string(),
        undo_token: Some(undo_token.clone()),
        ..intent
    };
    // A failure here means the ref already moved but no completed record (and so
    // no safe undo path) exists. Roll the ref back rather than leave an
    // un-undoable rewrite that misreports as a generic error. drop은 예외다:
    // ref와 워킹트리가 이미 새 tip에 일치하므로, ref만 되돌리면 워킹트리와
    // 어긋난 더 나쁜 상태가 된다 — partial failure로 정직하게 보고한다.
    if let Err(err) = write_record_replace(&record_path, &completed) {
        return Err(match &drop_prediction {
            Some(_) => {
                sync_partial_failure(&git, &worktree_root, &branch_ref, &record_path, true, err)
            }
            None => rollback(
                &git,
                &worktree_root,
                &branch_ref,
                &previous_tip,
                &new_tip,
                &record_path,
                err,
            ),
        });
    }

    Ok(ExecuteResult {
        schema_version: EXECUTE_SCHEMA_VERSION.to_string(),
        plan_id: fresh.plan_id,
        action: ACTION_HISTORY_EDIT.to_string(),
        repository: worktree_root,
        executed: true,
        effects,
        undo_token: Some(ExecuteUndoToken::HistoryEdit(Box::new(undo_token))),
    })
}

fn validate_static_contract(plan: &HistoryEditPlan) -> Result<()> {
    if plan.schema_version != HISTORY_EDIT_PLAN_SCHEMA_VERSION {
        return invalid_plan(
            "unsupported_schema_version",
            "history_edit execute requires super-git.plan.v0.4",
        );
    }
    let expected_plan_id = compute_history_edit_plan_id(plan);
    ensure_match("plan_id", &plan.plan_id, &expected_plan_id)?;

    if plan.action.kind != ACTION_HISTORY_EDIT {
        return invalid_plan(
            "unsupported_action",
            "history_edit execute supports only history_edit plans",
        );
    }
    let status = plan.execution.status.as_str();
    if status != "executable" && status != "preview_only" {
        return invalid_plan(
            "not_executable",
            "history_edit execute requires execution.status of executable or preview_only",
        );
    }
    // drop(tree-changing) plan은 replay 예측을 반드시 동반한다: prediction이
    // execute의 final_tree 오라클이므로, 둘 중 하나만 있는 plan은 위조다.
    let has_drop_instruction = plan
        .instructions
        .as_ref()
        .is_some_and(|instructions| instructions.items.iter().any(|item| item.op == "drop"));
    match (&plan.prediction, has_drop_instruction) {
        (None, false) => {}
        (Some(prediction), true) => validate_drop_contract(prediction, status)?,
        (Some(_), false) => {
            return invalid_plan(
                "prediction_unexpected",
                "history_edit plans without a drop instruction must not carry a replay prediction",
            );
        }
        (None, true) => {
            return invalid_plan(
                "prediction_required",
                "history_edit drop plans must carry the kept-commit replay prediction",
            );
        }
    }
    if !plan.execution.execute_supported
        || plan.execution.raw_git_allowed
        || !plan.execution.blocked_reasons.is_empty()
    {
        return invalid_plan(
            "unsupported_execution_contract",
            "history_edit execute requires an unblocked super-git-only plan",
        );
    }
    // drop execute는 워킹트리를 동기화하므로 undo도 대칭 kind를 요구한다.
    let expected_undo_kind = if plan.prediction.is_some() {
        "restore_branch_tip_and_worktree"
    } else {
        "restore_branch_tip_snapshot"
    };
    if plan.undo_strategy.kind != expected_undo_kind {
        return invalid_plan(
            "unsupported_undo_strategy",
            &format!(
                "history_edit execute requires {expected_undo_kind} undo strategy for this plan"
            ),
        );
    }
    if plan.branch.is_none() {
        return invalid_plan(
            "branch_required",
            "history_edit execute requires an attached branch",
        );
    }
    if plan.instructions.is_none() || plan.result_summary.is_none() {
        return invalid_plan(
            "instructions_required",
            "history_edit execute requires resolved instructions and a result summary",
        );
    }
    validate_tier_contract(plan, status)
}

/// drop plan이 execute에 들어올 수 있는 유일한 모양: clean replay 예측과
/// final_tree 오라클을 갖춘 항상-confirmation-gated(preview_only) plan.
/// conflicted/오라클 없는 plan은 fail-closed — preview가 그런 plan을 만들지
/// 않지만, execute는 plan 파일을 신뢰하지 않는다.
fn validate_drop_contract(prediction: &HistoryEditPrediction, status: &str) -> Result<()> {
    if prediction.kind != "kept_commit_replay" {
        return invalid_plan(
            "prediction_kind_unsupported",
            "history_edit execute supports only kept_commit_replay predictions",
        );
    }
    if prediction.status != "clean" {
        return invalid_plan(
            "prediction_not_clean",
            "a drop plan with a predicted conflict can never execute; resolve the instruction list instead",
        );
    }
    if prediction.final_tree.is_none() {
        return invalid_plan(
            "prediction_final_tree_missing",
            "a tree-changing plan without a final_tree oracle is never executable",
        );
    }
    if status != "preview_only" {
        return invalid_plan(
            "unsupported_execution_contract",
            "drop plans are always confirmation-gated preview_only plans",
        );
    }
    Ok(())
}

/// Risk and confirmation expectations differ by tier: unpublished plans are
/// medium-risk and carry no confirmation block; published plans are high-risk
/// and require one. The confirmation block's fields are bound by plan_id, so a
/// light presence check here plus the artifact validation closes the contract.
fn validate_tier_contract(plan: &HistoryEditPlan, status: &str) -> Result<()> {
    match status {
        "executable" => {
            if plan.execution.requires_confirmation_artifact || plan.confirmation.is_some() {
                return invalid_plan(
                    "unexpected_confirmation_block",
                    "executable history_edit plans must not require or carry a confirmation block",
                );
            }
            if plan.risk.severity != "medium"
                || plan.risk.reversibility != "reversible_if_unchanged"
                || plan.risk.requires_human_confirmation
            {
                return invalid_plan(
                    "unsupported_risk",
                    "executable history_edit requires medium reversible_if_unchanged risk without human confirmation",
                );
            }
            Ok(())
        }
        "preview_only" => {
            if !plan.execution.requires_confirmation_artifact {
                return invalid_plan(
                    "unsupported_execution_contract",
                    "preview_only history_edit must require a confirmation artifact",
                );
            }
            if plan.risk.severity != "high"
                || plan.risk.reversibility != "reversible_if_unchanged"
                || !plan.risk.requires_human_confirmation
            {
                return invalid_plan(
                    "unsupported_risk",
                    "preview_only history_edit requires high reversible_if_unchanged risk with human confirmation",
                );
            }
            match &plan.confirmation {
                Some(confirmation) if confirmation.required_before_execute => Ok(()),
                _ => invalid_plan(
                    "confirmation_block_required",
                    "preview_only history_edit must carry a confirmation block that requires confirmation",
                ),
            }
        }
        _ => invalid_plan("not_executable", "unreachable execution status"),
    }
}

fn parse_confirmation(bytes: &[u8]) -> Result<HistoryEditConfirmation> {
    let value: Value = serde_json::from_slice(bytes)?;
    if value.get("schema_version").and_then(Value::as_str) != Some(CONFIRMATION_SCHEMA_VERSION) {
        return invalid_plan(
            "confirmation_schema_unsupported",
            "history_edit confirmation requires super-git.confirmation.v0.1",
        );
    }
    serde_json::from_value(value).map_err(|err| SuperGitError::ExecutePlanInvalid {
        code: "confirmation_artifact_invalid".to_string(),
        message: format!("history_edit confirmation artifact is invalid JSON shape: {err}"),
    })
}

/// Static validation mirrors the C7-C rule table with history_edit identity
/// fields. The artifact is authorization, never a substitute for the fresh
/// revalidation that follows; a forged or stale confirmation cannot execute.
fn validate_confirmation(
    plan: &HistoryEditPlan,
    confirmation: &HistoryEditConfirmation,
) -> Result<()> {
    let branch = plan
        .branch
        .as_ref()
        .expect("branch presence validated by static contract");
    let plan_confirmation = plan
        .confirmation
        .as_ref()
        .expect("confirmation block validated by static contract");

    if confirmation.kind.as_deref() != Some("destructive_action_confirmation") {
        return invalid_plan(
            "confirmation_kind_unsupported",
            "history_edit confirmation kind must be destructive_action_confirmation",
        );
    }
    if confirmation.action.as_deref() != Some(plan.action.kind.as_str()) {
        return invalid_plan(
            "confirmation_action_mismatch",
            "history_edit confirmation action must match the plan action",
        );
    }
    if confirmation.plan_schema_version.as_deref() != Some(plan.schema_version.as_str())
        || confirmation.plan_id.as_deref() != Some(plan.plan_id.as_str())
    {
        return invalid_plan(
            "confirmation_plan_mismatch",
            "history_edit confirmation must match the plan schema version and plan id",
        );
    }

    let Some(target) = &confirmation.target else {
        return invalid_plan(
            "confirmation_target_mismatch",
            "history_edit confirmation target is required",
        );
    };
    if target.branch_ref.as_deref() != Some(branch.ref_name.as_str())
        || target.tip_commit.as_deref() != Some(branch.tip_commit.as_str())
        || target.git_common_dir.as_deref() != Some(plan.repository.git_common_dir.as_path())
    {
        return invalid_plan(
            "confirmation_target_mismatch",
            "history_edit confirmation target must match the plan branch, tip, and git_common_dir",
        );
    }

    if confirmation.acknowledged_reason_codes.as_ref() != Some(&plan_confirmation.reason_codes) {
        return invalid_plan(
            "confirmation_reason_codes_mismatch",
            "history_edit confirmation must acknowledge the exact plan reason codes",
        );
    }
    if confirmation.acknowledged_undo_strategy.as_deref() != Some(plan.undo_strategy.kind.as_str())
    {
        return invalid_plan(
            "confirmation_undo_strategy_mismatch",
            "history_edit confirmation must acknowledge the plan undo strategy",
        );
    }

    let Some(acknowledgement) = &confirmation.acknowledgement else {
        return invalid_plan(
            "confirmation_acknowledgement_missing",
            "history_edit confirmation must include an explicit acknowledgement",
        );
    };
    if acknowledgement.method.as_deref() != Some("cli_typed_phrase") {
        return invalid_plan(
            "confirmation_method_unsupported",
            "history_edit confirmation acknowledgement method must be cli_typed_phrase",
        );
    }
    // 한 plan에 phrase는 하나(preview와 같은 규칙): drop이 있으면 drop phrase가
    // published phrase를 대신한다. dropped 수는 plan_id가 바인딩하는 prediction
    // 에서 온다.
    let expected_phrase = match &plan.prediction {
        Some(prediction) => preview_history_edit::drop_confirmation_phrase(
            prediction.dropped_commits.len(),
            &branch.ref_name,
            &branch.tip_commit,
        ),
        None => preview_history_edit::confirmation_phrase(&branch.ref_name, &branch.tip_commit),
    };
    if acknowledgement.phrase.as_deref() != Some(expected_phrase.as_str()) {
        return invalid_plan(
            "confirmation_phrase_mismatch",
            "history_edit confirmation phrase must match the plan's deterministic phrase",
        );
    }
    Ok(())
}

/// Re-derive the plan from fresh repository state and require an identical plan
/// id. This binds branch tip, range commits, published status, and instructions
/// in one check: any drift since preview changes the id and aborts. The fresh
/// plan is returned so the write path runs off authentic, live-read fields
/// rather than the caller-supplied plan's (forgeable) advisory copies.
fn validate_fresh_binding(current_path: &Path, plan: &HistoryEditPlan) -> Result<HistoryEditPlan> {
    let instructions_bytes = reconstruct_instructions_bytes(plan)?;
    let fresh = preview_history_edit(
        current_path,
        &plan.action.options.base,
        Some(&instructions_bytes),
    )?;
    // The fresh tier must match the plan's: a range that became (or stopped
    // being) published since preview shifts the status and the plan_id, and is
    // rejected rather than executed under stale assumptions.
    if fresh.execution.status != plan.execution.status {
        return mismatch(
            "fresh_execution_status",
            &plan.execution.status,
            &fresh.execution.status,
        );
    }
    // drop: fresh replay 예측이 plan의 오라클과 달라졌으면 plan_id mismatch에
    // 묻히기 전에 정확한 진단을 준다(예: git 버전 교체로 병합 결과 드리프트).
    if let (Some(planned), Some(fresh_prediction)) = (&plan.prediction, &fresh.prediction) {
        if fresh_prediction.final_tree != planned.final_tree {
            return mismatch(
                "fresh_prediction.final_tree",
                planned.final_tree.as_deref().unwrap_or("none"),
                fresh_prediction.final_tree.as_deref().unwrap_or("none"),
            );
        }
    }
    ensure_match("plan_id", &plan.plan_id, &fresh.plan_id)?;
    Ok(fresh)
}

fn reconstruct_instructions_bytes(plan: &HistoryEditPlan) -> Result<Vec<u8>> {
    let instructions =
        plan.instructions
            .as_ref()
            .ok_or_else(|| SuperGitError::ExecutePlanInvalid {
                code: "instructions_required".to_string(),
                message: "executable history_edit plans must carry resolved instructions"
                    .to_string(),
            })?;
    let items = instructions
        .items
        .iter()
        .map(|item| {
            let mut object = serde_json::Map::new();
            object.insert("commit".to_string(), serde_json::json!(item.commit));
            object.insert("op".to_string(), serde_json::json!(item.op));
            if let Some(message) = &item.message {
                object.insert("message".to_string(), serde_json::json!(message));
            }
            serde_json::Value::Object(object)
        })
        .collect::<Vec<_>>();
    let document = serde_json::json!({
        "schema_version": instructions.schema_version,
        "action": ACTION_HISTORY_EDIT,
        "base": plan.action.options.base,
        "items": items,
    });
    Ok(serde_json::to_vec(&document)?)
}

/// One rebuilt commit: the primary instruction plus its consecutive fold chain.
struct RebuildGroup {
    primary: String,
    /// Tree comes from the last original commit folded in, which already holds
    /// every earlier change in the chain, so the final tree stays identical.
    last_commit: String,
    author_name: String,
    author_email: String,
    author_date: String,
    final_message: String,
    changed: bool,
}

/// Returns the rebuild groups plus the group count before the first `drop`
/// (None when nothing is dropped) — the unchanged-prefix cap for replay.
fn build_groups(plan: &HistoryEditPlan) -> Result<(Vec<RebuildGroup>, Option<usize>)> {
    let instructions = plan
        .instructions
        .as_ref()
        .expect("validated executable plan carries instructions");
    let by_oid: HashMap<&str, &HistoryEditPlanCommit> = plan
        .range
        .commits
        .iter()
        .map(|commit| (commit.commit.as_str(), commit))
        .collect();

    let mut groups: Vec<RebuildGroup> = Vec::new();
    let mut groups_before_first_drop: Option<usize> = None;
    for item in &instructions.items {
        let commit = by_oid.get(item.commit.as_str()).copied().ok_or_else(|| {
            SuperGitError::ExecutePlanInvalid {
                code: "instruction_commit_not_in_range".to_string(),
                message: format!(
                    "instruction commit {} is not in the plan range",
                    item.commit
                ),
            }
        })?;
        match item.op.as_str() {
            "pick" => groups.push(RebuildGroup {
                primary: commit.commit.clone(),
                last_commit: commit.commit.clone(),
                author_name: commit.author_name.clone(),
                author_email: commit.author_email.clone(),
                author_date: commit.author_date.clone(),
                final_message: commit.message.clone(),
                changed: false,
            }),
            "reword" => groups.push(RebuildGroup {
                primary: commit.commit.clone(),
                last_commit: commit.commit.clone(),
                author_name: commit.author_name.clone(),
                author_email: commit.author_email.clone(),
                author_date: commit.author_date.clone(),
                final_message: instruction_message(item)?,
                changed: true,
            }),
            "squash" => {
                let group = last_group(&mut groups)?;
                group.last_commit = commit.commit.clone();
                group.final_message = instruction_message(item)?;
                group.changed = true;
            }
            "fixup" => {
                let group = last_group(&mut groups)?;
                group.last_commit = commit.commit.clone();
                group.changed = true;
            }
            // drop은 결과 그룹을 만들지 않는다: 이 커밋의 patch가 최종
            // history에서 빠진다. fold와의 혼용은 preview가 차단하고 fresh
            // binding이 재확인하므로 여기서는 위치만 기록한다.
            "drop" => {
                groups_before_first_drop.get_or_insert(groups.len());
            }
            other => {
                return invalid_plan(
                    "unsupported_instruction_op",
                    &format!("history_edit execute does not support op {other}"),
                );
            }
        }
    }
    Ok((groups, groups_before_first_drop))
}

fn last_group(groups: &mut [RebuildGroup]) -> Result<&mut RebuildGroup> {
    groups
        .last_mut()
        .ok_or_else(|| SuperGitError::ExecutePlanInvalid {
            code: "instruction_fold_without_predecessor".to_string(),
            message: "a fold instruction has no preceding commit".to_string(),
        })
}

fn instruction_message(item: &crate::model::HistoryEditPlanInstructionItem) -> Result<String> {
    item.message
        .clone()
        .ok_or_else(|| SuperGitError::ExecutePlanInvalid {
            code: "instruction_message_missing".to_string(),
            message: format!("instruction {} requires a message", item.op),
        })
}

fn rebuild_commits(
    git: &Git,
    worktree_root: &Path,
    base_commit: &str,
    groups: &[RebuildGroup],
) -> Result<String> {
    // Leading unchanged picks keep their original object ids, exactly like
    // interactive rebase, so only history from the first edit onward is new.
    let unchanged_prefix = groups.iter().take_while(|group| !group.changed).count();
    let mut parent = if unchanged_prefix == 0 {
        base_commit.to_string()
    } else {
        groups[unchanged_prefix - 1].primary.clone()
    };

    for group in &groups[unchanged_prefix..] {
        let tree = read_tree_oid(git, worktree_root, &group.last_commit)?;
        parent = commit_tree(git, worktree_root, &tree, &parent, group)?;
    }
    Ok(parent)
}

/// drop 경로의 chain 재구축: 각 kept 커밋의 tree는 원래 tree가 아니라 fresh
/// 예측이 검증한 step별 merged_tree에서 온다(merge 계산을 반복하지 않는다).
/// 전부 drop이면 새 tip은 base 그 자체다.
fn rebuild_replayed_commits(
    git: &Git,
    worktree_root: &Path,
    base_commit: &str,
    groups: &[RebuildGroup],
    steps: &[HistoryEditPredictionStep],
    groups_before_first_drop: Option<usize>,
) -> Result<String> {
    // 예측 step과 kept 그룹은 1:1이어야 한다(fold는 drop과 혼용 불가이므로
    // 그룹 = 단일 커밋). 어긋나면 위조된 plan이거나 구현 버그다.
    if steps.len() != groups.len() {
        return invalid_plan(
            "prediction_steps_mismatch",
            &format!(
                "prediction has {} steps but the plan keeps {} commits",
                steps.len(),
                groups.len()
            ),
        );
    }
    for (group, step) in groups.iter().zip(steps) {
        if step.commit != group.primary || step.status != "clean" {
            return invalid_plan(
                "prediction_steps_mismatch",
                &format!(
                    "prediction step for {} does not match kept commit {} with a clean status",
                    step.commit, group.primary
                ),
            );
        }
    }

    // 첫 drop 앞의 unchanged pick들만 원래 oid를 유지한다: 첫 drop부터는
    // parent가 달라지므로 이후 전부가 새 커밋이다.
    let unchanged_prefix = groups
        .iter()
        .take_while(|group| !group.changed)
        .count()
        .min(groups_before_first_drop.unwrap_or(usize::MAX));
    let mut parent = if unchanged_prefix == 0 {
        base_commit.to_string()
    } else {
        groups[unchanged_prefix - 1].primary.clone()
    };

    for (group, step) in groups.iter().zip(steps).skip(unchanged_prefix) {
        parent = commit_tree(git, worktree_root, &step.merged_tree, &parent, group)?;
    }
    Ok(parent)
}

fn commit_tree(
    git: &Git,
    worktree_root: &Path,
    tree: &str,
    parent: &str,
    group: &RebuildGroup,
) -> Result<String> {
    let envs = [
        ("GIT_AUTHOR_NAME", group.author_name.as_str()),
        ("GIT_AUTHOR_EMAIL", group.author_email.as_str()),
        ("GIT_AUTHOR_DATE", group.author_date.as_str()),
    ];
    let output = git.run_write_in_with_env_stdin(
        worktree_root,
        ["commit-tree", tree, "-p", parent],
        &envs,
        group.final_message.as_bytes(),
    )?;
    let oid = output.stdout.trim().to_string();
    if oid.is_empty() {
        return invalid_plan(
            "commit_tree_empty_output",
            "git commit-tree returned no object id",
        );
    }
    Ok(oid)
}

fn compare_and_swap_ref(
    git: &Git,
    worktree_root: &Path,
    branch_ref: &str,
    new_oid: &str,
    old_oid: &str,
) -> Result<()> {
    git.run_write_in(worktree_root, ["update-ref", branch_ref, new_oid, old_oid])
        .map_err(|err| SuperGitError::ExecutePreconditionMismatch {
            field: "branch_ref_compare_and_swap".to_string(),
            expected: format!("{branch_ref}=={old_oid}"),
            actual: format!("update-ref refused: {err}"),
        })?;
    Ok(())
}

fn post_verify(
    git: &Git,
    worktree_root: &Path,
    plan: &HistoryEditPlan,
    new_tip: &str,
    expected_tree: &str,
    commits_after: u32,
) -> Result<()> {
    let current_tip = read_ref_oid(git, worktree_root, &plan.branch.as_ref().unwrap().ref_name)?;
    if current_tip != new_tip {
        return mismatch("post_verify.branch_tip", new_tip, &current_tip);
    }
    let new_tree = read_tree_oid(git, worktree_root, new_tip)?;
    if new_tree != expected_tree {
        // tree-preserving ops must keep the original tree; drop must land
        // exactly on the predicted final_tree oracle.
        return mismatch("post_verify.tree_identity", expected_tree, &new_tree);
    }
    let count = count_range_commits(git, worktree_root, &plan.range.base_commit, new_tip)?;
    if count != commits_after {
        return mismatch(
            "post_verify.commit_count",
            &commits_after.to_string(),
            &count.to_string(),
        );
    }
    Ok(())
}

/// Restore the branch to its pre-edit tip after a failure that happened once the
/// ref had already moved. On a successful rollback the intent record is removed
/// so the still-valid plan can be re-executed; on a failed rollback the record
/// is kept as the provenance anchor and `ExecuteRollbackFailed` surfaces both
/// errors so the caller knows the ref is still at `new_tip`.
fn rollback(
    git: &Git,
    worktree_root: &Path,
    branch_ref: &str,
    previous_tip: &str,
    new_tip: &str,
    record_path: &Path,
    original: SuperGitError,
) -> SuperGitError {
    match compare_and_swap_ref(git, worktree_root, branch_ref, previous_tip, new_tip) {
        Ok(()) => {
            let _ = fs::remove_file(record_path);
            original
        }
        Err(rollback_err) => SuperGitError::ExecuteRollbackFailed {
            original_error: original.to_string(),
            rollback_error: rollback_err.to_string(),
        },
    }
}

fn success_effects(plan: &HistoryEditPlan, branch_ref: &str, new_tip: &str) -> Vec<String> {
    let summary = plan
        .result_summary
        .as_ref()
        .expect("validated executable plan carries a result summary");
    if let Some(prediction) = &plan.prediction {
        return vec![
            format!(
                "Rewrote {} commits on {} into {} commits at {}, removing {} commit's patch(es) from the final history.",
                summary.commits_before,
                branch_ref,
                summary.commits_after,
                new_tip,
                prediction.dropped_commits.len()
            ),
            "Preserved every author identity on the kept commits.".to_string(),
            "Synchronized the index and working tree to the new tip.".to_string(),
        ];
    }
    vec![
        format!(
            "Rewrote {} commits on {} into {} commits at {}.",
            summary.commits_before, branch_ref, summary.commits_after, new_tip
        ),
        "Preserved every author identity and the final tree; the working tree and index are unchanged."
            .to_string(),
    ]
}

fn read_ref_oid(git: &Git, worktree_root: &Path, reference: &str) -> Result<String> {
    let output = git.run_in(worktree_root, ["rev-parse", "--verify", reference])?;
    Ok(output.stdout.trim().to_string())
}

fn read_tree_oid(git: &Git, worktree_root: &Path, commit: &str) -> Result<String> {
    let output = git.run_in(
        worktree_root,
        ["rev-parse", "--verify", &format!("{commit}^{{tree}}")],
    )?;
    Ok(output.stdout.trim().to_string())
}

fn count_range_commits(git: &Git, worktree_root: &Path, base: &str, tip: &str) -> Result<u32> {
    let output = git.run_in(
        worktree_root,
        ["rev-list", "--count", &format!("{base}..{tip}")],
    )?;
    output
        .stdout
        .trim()
        .parse::<u32>()
        .map_err(|_| SuperGitError::ExecutePreconditionMismatch {
            field: "post_verify.commit_count".to_string(),
            expected: "a number".to_string(),
            actual: output.stdout.trim().to_string(),
        })
}

pub(crate) fn read_status_signature(git: &Git, worktree_root: &Path) -> Result<String> {
    // Pin the untracked mode so the drift comparison is independent of
    // status.showUntrackedFiles config.
    let output = git.run_in(
        worktree_root,
        ["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    Ok(output.stdout)
}

/// drop execute의 하드 게이트(C8-drop 계약): sync가 사용자 변경을 덮어쓸 수
/// 없도록 tracked 변경과 untracked 모두 없어야 한다. ignored 파일은 status에
/// 잡히지 않아 이 게이트가 보지 못한다 — 새 tip의 tracked 경로와 충돌하는
/// ignored 경로는 `ensure_no_ignored_path_collisions`가 따로 막는다.
fn ensure_clean_working_tree(git: &Git, worktree_root: &Path) -> Result<()> {
    let status = read_status_signature(git, worktree_root)?;
    if status.is_empty() {
        return Ok(());
    }
    mismatch(
        "working_tree_clean",
        "clean working tree and index (including untracked)",
        &format!("{} dirty or untracked path(s)", status.lines().count()),
    )
}

/// drop sync의 ignored-경로 빈틈을 막는다: `read-tree -u --reset`은 새 tip이
/// tracked로 부활시키는 경로 위의 ignored untracked 파일을 조용히 덮어쓴다
/// (예: 한 커밋이 force-add했던 ignored 파일을 지운 커밋을 drop하는 경우 —
/// 스파이크 실측). 모든 ignored를 막으면 node_modules 같은 정상 사용이
/// 깨지므로, 새 tip의 tracked 경로와 충돌하는 ignored 경로만 hard block한다.
/// 반드시 CAS ref move 전에 실행되어 실패가 어떤 상태도 바꾸지 않는다.
fn ensure_no_ignored_path_collisions(git: &Git, worktree_root: &Path, new_tip: &str) -> Result<()> {
    let collisions = ignored_path_collisions(git, worktree_root, new_tip)?;
    if collisions.is_empty() {
        return Ok(());
    }
    mismatch(
        "ignored_path_collision",
        "no ignored paths colliding with paths tracked in the new tip",
        &describe_collisions(&collisions),
    )
}

/// 대상 tip의 tracked 경로와 충돌하는 ignored untracked 경로들(표시용 lossy
/// 문자열, 정렬). drop execute(새 tip 기준)와 drop undo(pre-execute tip 기준)
/// 가 같은 검사를 공유한다. 충돌의 세 모양: 경로 일치, ignored 파일이 대상
/// tracked 디렉터리 자리를 차지, ignored 디렉터리가 대상 tracked 파일 자리를
/// 차지. **빈** ignored 디렉터리는 의도적으로 잡지 않는다: `ls-files -o`에
/// 나타나지 않고, 내용물이 없어 잃을 데이터가 없으며, `read-tree -u`가 git의
/// 일반 checkout 의미론대로 자유로이 제거한다(스파이크 실측) — 내용이 생기는
/// 순간 그 파일들이 prefix 규칙에 걸린다.
pub(crate) fn ignored_path_collisions(
    git: &Git,
    worktree_root: &Path,
    target_tip: &str,
) -> Result<Vec<String>> {
    let ignored_raw = git.run_bytes_in(
        worktree_root,
        ["ls-files", "-i", "-o", "--exclude-standard", "-z"],
    )?;
    let ignored = split_nul(&ignored_raw.stdout);
    if ignored.is_empty() {
        return Ok(Vec::new());
    }

    let tracked_raw = git.run_bytes_in(
        worktree_root,
        ["ls-tree", "-r", "-z", "--name-only", target_tip],
    )?;
    let tracked: HashSet<&[u8]> = split_nul(&tracked_raw.stdout).into_iter().collect();
    let mut tracked_dirs: HashSet<&[u8]> = HashSet::new();
    for path in &tracked {
        tracked_dirs.extend(dir_prefixes(path));
    }

    let mut collisions: Vec<String> = ignored
        .into_iter()
        .filter(|path| {
            tracked.contains(path)
                || tracked_dirs.contains(path)
                || dir_prefixes(path).any(|prefix| tracked.contains(prefix))
        })
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect();
    collisions.sort();
    Ok(collisions)
}

pub(crate) fn describe_collisions(collisions: &[String]) -> String {
    let shown = collisions
        .iter()
        .take(5)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    format!("{} colliding path(s): {shown}", collisions.len())
}

/// "a/b/c" → "a", "a/b" (경로의 모든 디렉터리 prefix, 바이트 단위 — `-z`
/// 출력은 인코딩을 보장하지 않으므로 String 변환 없이 비교한다).
fn dir_prefixes<'a>(path: &'a [u8]) -> impl Iterator<Item = &'a [u8]> + 'a {
    path.iter()
        .enumerate()
        .filter(|(_, byte)| **byte == b'/')
        .map(move |(index, _)| &path[..index])
}

fn split_nul(bytes: &[u8]) -> Vec<&[u8]> {
    bytes
        .split(|byte| *byte == b'\0')
        .filter(|path| !path.is_empty())
        .collect()
}

/// C8-drop 계약의 sync primitive: `read-tree -u --reset`은 ref를 건드리지
/// 않고(hook도 없음) index와 워킹트리만 새 tip으로 맞춘다. 끝난 뒤 status가
/// 비어 있는지까지가 sync다 — 부분 동기화를 성공으로 보고하지 않는다.
pub(crate) fn sync_working_tree(git: &Git, worktree_root: &Path, new_tip: &str) -> Result<()> {
    git.run_write_in(worktree_root, ["read-tree", "-u", "--reset", new_tip])?;
    let status = read_status_signature(git, worktree_root)?;
    if !status.is_empty() {
        return mismatch(
            "sync_verify.working_tree_clean",
            "a clean working tree at the new tip",
            &format!("{} path(s) still differ", status.lines().count()),
        );
    }
    Ok(())
}

/// ref가 이미 새 tip으로 움직인 뒤의 실패 보고. ref 롤백은 하지 않는다 —
/// ref는 올바르고, 미완인 것은 sync 또는 record뿐이다(계약 문서의
/// partial-failure 창). record가 intent로 남아 undo와 재실행이 fail-closed다.
fn sync_partial_failure(
    git: &Git,
    worktree_root: &Path,
    branch_ref: &str,
    record_path: &Path,
    sync_completed: bool,
    original: SuperGitError,
) -> SuperGitError {
    let observed_tip =
        read_ref_oid(git, worktree_root, branch_ref).unwrap_or_else(|_| "unreadable".to_string());
    let safe_next = if sync_completed {
        "the branch ref and working tree are already at the new tip; only the execution record is incomplete, so automatic undo is unavailable — inspect the execution record before any manual repair"
    } else {
        "the branch ref is correctly at the new tip; the index and working tree may be partially synchronized — verify the working-tree state and finish synchronizing to the recorded new_tip (see the execution record); do not re-run execute"
    };
    SuperGitError::ExecuteSyncPartialFailure(Box::new(crate::error::SyncPartialFailureError {
        action: ACTION_HISTORY_EDIT.to_string(),
        message: original.to_string(),
        branch_ref: branch_ref.to_string(),
        observed_tip,
        sync_completed,
        execution_record_path: record_path.to_path_buf(),
        safe_next: safe_next.to_string(),
    }))
}

fn execution_record_path(plan: &HistoryEditPlan, branch_ref: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(b"super-git-history-edit-execute-v0.1\n");
    hasher.update(plan.plan_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(branch_ref.as_bytes());
    let id = format_hex_digest(hasher.finalize().as_slice());
    plan.repository
        .git_common_dir
        .join("super-git")
        .join("executions")
        .join(format!("{id}.json"))
}

fn write_record_create_new(path: &Path, record: &HistoryEditExecutionRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(record)?;
    let mut file = execute::create_new_or_already_attempted(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn write_record_replace(path: &Path, record: &HistoryEditExecutionRecord) -> Result<()> {
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

fn invalid_plan<T>(code: &str, message: &str) -> Result<T> {
    Err(SuperGitError::ExecutePlanInvalid {
        code: code.to_string(),
        message: message.to_string(),
    })
}

fn ensure_match(field: &str, expected: &str, actual: &str) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        mismatch(field, expected, actual)
    }
}

fn mismatch<T>(field: &str, expected: &str, actual: &str) -> Result<T> {
    Err(SuperGitError::ExecutePreconditionMismatch {
        field: field.to_string(),
        expected: expected.to_string(),
        actual: actual.to_string(),
    })
}

fn format_hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::rollback;
    use crate::git::command::Git;
    use crate::git::execute::create_new_or_already_attempted;
    use crate::SuperGitError;

    #[test]
    fn create_new_record_maps_already_exists_to_contract_error() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let path = tmp.path().join("intent.json");
        // Simulate a leftover record from a prior incomplete attempt.
        std::fs::write(&path, "{}").expect("write record");

        let err = create_new_or_already_attempted(&path).expect_err("must reject existing record");

        match err {
            SuperGitError::ExecutePlanInvalid { code, .. } => {
                assert_eq!(code, "execution_already_attempted");
            }
            other => panic!("expected ExecutePlanInvalid, got {other:?}"),
        }
    }

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

    fn commit(dir: &Path, file: &str, content: &str) {
        std::fs::write(dir.join(file), content).expect("write");
        git(dir, &["add", file]);
        git(dir, &["commit", "-q", "-m", content]);
    }

    /// Repo on `main` with three commits; returns (repo, c1, c2, c3).
    fn repo_with_three(tmp: &Path) -> (PathBuf, String, String, String) {
        let repo = tmp.join("repo");
        std::fs::create_dir_all(&repo).expect("create repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        commit(&repo, "a.txt", "a");
        let c1 = rev(&repo, "HEAD");
        commit(&repo, "b.txt", "b");
        let c2 = rev(&repo, "HEAD");
        commit(&repo, "c.txt", "c");
        let c3 = rev(&repo, "HEAD");
        (repo, c1, c2, c3)
    }

    fn write_record(repo: &Path, name: &str) -> PathBuf {
        let path = repo.join(".git/super-git/executions").join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, "{}").expect("write record");
        path
    }

    #[test]
    fn rollback_restores_branch_and_removes_record_on_success() {
        let tmp = tempfile::tempdir().expect("temp");
        let (repo, c1, c2, _c3) = repo_with_three(tmp.path());
        // Simulate execute having moved the branch to new_tip = c2.
        git(&repo, &["update-ref", "refs/heads/main", &c2]);
        let record = write_record(&repo, "intent.json");
        let original = SuperGitError::ExecutePlanInvalid {
            code: "original".to_string(),
            message: "post-write failure".to_string(),
        };

        let returned = rollback(
            &Git::default(),
            &repo,
            "refs/heads/main",
            &c1,
            &c2,
            &record,
            original,
        );

        assert_eq!(rev(&repo, "refs/heads/main"), c1, "ref restored to old tip");
        assert!(!record.exists(), "intent record removed after rollback");
        assert!(
            matches!(returned, SuperGitError::ExecutePlanInvalid { .. }),
            "original error is surfaced after a successful rollback"
        );
    }

    #[test]
    fn ignored_path_collision_catches_a_file_squatting_on_a_tracked_directory() {
        let tmp = tempfile::tempdir().expect("temp");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join(".gitignore"), "dir\n").expect("gitignore");
        std::fs::create_dir_all(repo.join("dir")).expect("mkdir");
        std::fs::write(repo.join("dir/file.txt"), "x").expect("write");
        git(&repo, &["add", "-f", ".gitignore", "dir/file.txt"]);
        git(&repo, &["commit", "-q", "-m", "c0"]);
        let tip = rev(&repo, "HEAD");
        // 워킹트리에서 tracked 디렉터리를 ignored **파일**로 바꿔치기: 새 tip이
        // dir/file.txt를 실체화하려면 그 파일이 자리를 비켜야 한다.
        std::fs::remove_dir_all(repo.join("dir")).expect("remove dir");
        std::fs::write(repo.join("dir"), "squatter").expect("write squatter");

        let err = super::ensure_no_ignored_path_collisions(&Git::default(), &repo, &tip)
            .expect_err("a file squatting on a tracked directory must collide");

        match err {
            SuperGitError::ExecutePreconditionMismatch { field, actual, .. } => {
                assert_eq!(field, "ignored_path_collision");
                assert!(
                    actual.contains("dir"),
                    "must name the colliding path: {actual}"
                );
            }
            other => panic!("expected ExecutePreconditionMismatch, got {other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(repo.join("dir")).expect("read"),
            "squatter",
            "the check must not touch anything"
        );
    }

    #[test]
    fn sync_working_tree_fails_before_changing_anything_on_index_lock() {
        let tmp = tempfile::tempdir().expect("temp");
        let (repo, c1, _c2, _c3) = repo_with_three(tmp.path());
        // 다른 프로세스가 index를 잡은 상황: read-tree는 락 단계에서 실패하고
        // index/워킹트리는 그대로여야 한다(계약 문서의 스파이크 실측 모드).
        std::fs::write(repo.join(".git/index.lock"), "").expect("lock");

        let err = super::sync_working_tree(&Git::default(), &repo, &c1).expect_err("must refuse");

        assert!(matches!(err, SuperGitError::GitCommandFailed { .. }));
        let status = run_git(
            &repo,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        );
        let stdout = String::from_utf8_lossy(&status.stdout);
        // index.lock 자체는 status에 안 잡힌다; 비어 있으면 부분 sync가 없다.
        assert_eq!(stdout.trim(), "", "no partial sync on lock failure");
        assert!(repo.join("c.txt").exists(), "working tree untouched");
    }

    #[test]
    fn sync_partial_failure_reports_observed_tip_without_rolling_back() {
        let tmp = tempfile::tempdir().expect("temp");
        let (repo, _c1, c2, _c3) = repo_with_three(tmp.path());
        // Simulate execute having moved the branch to new_tip = c2 already.
        git(&repo, &["update-ref", "refs/heads/main", &c2]);
        let record = write_record(&repo, "intent.json");
        let original = SuperGitError::ExecutePlanInvalid {
            code: "original".to_string(),
            message: "sync broke".to_string(),
        };

        let err = super::sync_partial_failure(
            &Git::default(),
            &repo,
            "refs/heads/main",
            &record,
            false,
            original,
        );

        assert_eq!(err.code(), "execute_partial_failure");
        match err {
            SuperGitError::ExecuteSyncPartialFailure(details) => {
                assert_eq!(details.action, "history_edit");
                assert_eq!(
                    details.observed_tip, c2,
                    "envelope reports the live ref tip"
                );
                assert!(!details.sync_completed);
                assert_eq!(details.execution_record_path, record);
                assert!(details.safe_next.contains("do not re-run execute"));
            }
            other => panic!("expected ExecuteSyncPartialFailure, got {other:?}"),
        }
        assert_eq!(
            rev(&repo, "refs/heads/main"),
            c2,
            "partial failure must not roll the ref back"
        );
        assert!(record.exists(), "record stays as the provenance anchor");
    }

    #[test]
    fn rollback_reports_failure_and_keeps_record_when_tip_moved() {
        let tmp = tempfile::tempdir().expect("temp");
        let (repo, c1, c2, c3) = repo_with_three(tmp.path());
        // The branch is at c3, but rollback expects new_tip = c2: the CAS refuses.
        let record = write_record(&repo, "intent.json");
        let original = SuperGitError::ExecutePlanInvalid {
            code: "original".to_string(),
            message: "post-write failure".to_string(),
        };

        let returned = rollback(
            &Git::default(),
            &repo,
            "refs/heads/main",
            &c1,
            &c2,
            &record,
            original,
        );

        assert_eq!(
            rev(&repo, "refs/heads/main"),
            c3,
            "ref untouched on failed rollback"
        );
        assert!(
            record.exists(),
            "record kept as provenance on failed rollback"
        );
        assert!(
            matches!(returned, SuperGitError::ExecuteRollbackFailed { .. }),
            "a refused rollback is reported as ExecuteRollbackFailed"
        );
    }
}
