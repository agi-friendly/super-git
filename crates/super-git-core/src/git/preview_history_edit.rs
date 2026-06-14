use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::git::command::Git;
use crate::git::conflict_prediction;
use crate::git::history_edit::{
    self, HistoryEditCommit, HistoryEditProgram, HistoryEditScan, InstructionValidation,
};
use crate::model::{
    ActionRisk, HistoryEditAction, HistoryEditBlockedReason, HistoryEditExecution,
    HistoryEditInstructionsTemplate, HistoryEditOptions, HistoryEditPlan, HistoryEditPlanBranch,
    HistoryEditPlanCommit, HistoryEditPlanInstructionItem, HistoryEditPlanInstructions,
    HistoryEditPlanRange, HistoryEditPlanRepository, HistoryEditPlanWarning,
    HistoryEditPrecondition, HistoryEditPrediction, HistoryEditPredictionStep,
    HistoryEditPublishedScan, HistoryEditReorderAdvisory, HistoryEditResultSummaryView,
    HistoryEditUndoPreview, HistoryEditUndoStrategy, PreviewConfirmation,
    WorktreeReferenceCommands, HISTORY_EDIT_INSTRUCTIONS_SCHEMA_VERSION,
    HISTORY_EDIT_PLAN_SCHEMA_VERSION,
};
use crate::Result;

const ACTION_HISTORY_EDIT: &str = "history_edit";

/// Instruction-level block codes. A non-empty intersection means the agent's
/// instruction list does not yet match the editable range.
const INSTRUCTION_BLOCK_CODES: &[&str] = &[
    "instruction_op_unsupported",
    "instructions_unknown_commit",
    "instructions_duplicate_commit",
    "instructions_incomplete",
    "instructions_order_mismatch",
    "instruction_fold_without_predecessor",
    "instruction_message_missing",
    "instruction_message_empty",
    "instruction_message_unexpected",
    "instructions_no_effective_change",
    "drop_with_fold_unsupported",
    "reorder_with_drop_unsupported",
    "reorder_with_fold_unsupported",
];

pub fn preview_history_edit(
    current_path: &Path,
    base: &str,
    instructions_bytes: Option<&[u8]>,
) -> Result<HistoryEditPlan> {
    let scan = history_edit::scan_history_edit_range(current_path, base)?;

    // Only malformed or wrong-schema instruction input is a hard error. Content
    // problems (unknown commit, wrong order) become blocked plans so an agent
    // can repair its list from the plan alone.
    let mut has_drop = false;
    let validation = match instructions_bytes {
        Some(bytes) => {
            let document = history_edit::parse_instructions(bytes)?;
            has_drop = document.items.iter().any(|item| item.op == "drop");
            Some(history_edit::validate_instructions(
                &scan.range.commits,
                &document,
            ))
        }
        None => None,
    };

    let has_reorder = validation
        .as_ref()
        .and_then(|validation| validation.program.as_ref())
        .is_some_and(|program| program_reorders(&scan.range.commits, program));

    // drop/reorder는 replay 예측이 plan의 일부가 된다. scan/instruction block이
    // 하나라도 있으면 예측하지 않는다: merge-in-range 같은 block에서는 parent
    // 매핑 자체가 성립하지 않고, op-mixing block 위의 예측은 소음이다.
    let prediction = match (&validation, has_drop || has_reorder) {
        (Some(validation), true) if scan.blocks.is_empty() && validation.blocks.is_empty() => {
            validation
                .program
                .as_ref()
                .map(|program| {
                    let kind = if has_reorder {
                        "reordered_commit_replay"
                    } else {
                        "kept_commit_replay"
                    };
                    predict_history_replay(&scan, program, kind)
                })
                .transpose()?
        }
        _ => None,
    };

    Ok(build_plan(base, scan, validation, has_drop, prediction))
}

/// kept/reordered 커밋들을 base 위에 replay 예측한다 (C8-drop/C8-reorder 계약).
/// step별 3-way base는 각 커밋의 원래 parent다 — 드랍된 커밋일 수 있고,
/// 그것이 올바른 cherry-pick 의미다(재생되는 patch는 parent..commit).
///
/// `predict_replay_onto`에 위임하므로 preview의 read-only 경계는 정밀하다:
/// refs/index/워킹트리/설정은 안 건드리지만, clean step마다 참조되지 않는
/// (gc 회수 가능) synthetic commit을 object DB에 쓴다(`predict rebase`와 동일).
fn predict_history_replay(
    scan: &HistoryEditScan,
    program: &HistoryEditProgram,
    kind: &str,
) -> Result<HistoryEditPrediction> {
    // 선형 범위(oldest first)에서 i번째 커밋의 parent는 i-1번째, 첫 커밋의
    // parent는 base다. merge commit은 scan block으로 이미 걸러져 있다.
    let mut parent_of = std::collections::HashMap::new();
    let mut previous = scan.range.base_commit.as_str();
    for commit in &scan.range.commits {
        parent_of.insert(commit.commit.as_str(), previous.to_string());
        previous = commit.commit.as_str();
    }

    let mut dropped_commits = Vec::new();
    let mut kept: Vec<(String, String)> = Vec::new();
    for step in &program.steps {
        let parent = parent_of
            .get(step.commit.as_str())
            .expect("validated instruction commit is in range")
            .clone();
        if step.op == "drop" {
            dropped_commits.push(step.commit.clone());
        } else {
            kept.push((step.commit.clone(), parent));
        }
    }

    let git = Git::default();
    let outcome = conflict_prediction::predict_replay_onto(
        &git,
        &scan.repository.worktree_root,
        &scan.range.base_commit,
        &kept,
    )?;

    let steps = outcome
        .steps
        .into_iter()
        .map(|step| HistoryEditPredictionStep {
            commit: step.commit,
            parent: step.parent,
            status: step.prediction.status,
            merged_tree: step.prediction.merged_tree,
            conflicted_files: step.prediction.conflicted_files,
        })
        .collect();

    Ok(HistoryEditPrediction {
        kind: kind.to_string(),
        status: if outcome.final_tree.is_some() {
            "clean"
        } else {
            "conflicted"
        }
        .to_string(),
        dropped_commits,
        final_tree: outcome.final_tree,
        steps,
    })
}

fn program_reorders(range_commits: &[HistoryEditCommit], program: &HistoryEditProgram) -> bool {
    program.steps.len() == range_commits.len()
        && program
            .steps
            .iter()
            .zip(range_commits)
            .any(|(step, commit)| step.commit != commit.commit)
}

fn reorder_advisory(
    range_commits: &[HistoryEditCommit],
    program: &HistoryEditProgram,
) -> Option<HistoryEditReorderAdvisory> {
    if !program_reorders(range_commits, program) {
        return None;
    }

    let old_order: Vec<String> = range_commits
        .iter()
        .map(|commit| commit.commit.clone())
        .collect();
    let new_order: Vec<String> = program
        .steps
        .iter()
        .map(|step| step.commit.clone())
        .collect();
    let commits_reordered = old_order
        .iter()
        .zip(&new_order)
        .filter(|(old, new)| old != new)
        .count() as u32;

    Some(HistoryEditReorderAdvisory {
        commits_reordered,
        old_order,
        new_order,
    })
}

fn read_tree_oid(scan: &HistoryEditScan, commit: &str) -> Result<String> {
    let output = Git::default().run_in(
        &scan.repository.worktree_root,
        ["rev-parse", "--verify", &format!("{commit}^{{tree}}")],
    )?;
    Ok(output.stdout.trim().to_string())
}

fn prediction_blocks(
    scan: &HistoryEditScan,
    prediction: Option<&HistoryEditPrediction>,
    has_reorder: bool,
    old_tree: Option<&str>,
) -> Vec<history_edit::HistoryEditBlock> {
    let Some(prediction) = prediction else {
        return Vec::new();
    };

    if prediction.status == "conflicted" {
        let step = prediction
            .steps
            .iter()
            .find(|step| step.status == "conflicted")
            .expect("conflicted prediction has a conflicting step");
        return vec![history_edit::HistoryEditBlock {
            code: "predicted_conflict".to_string(),
            severity: "hard_block".to_string(),
            details: Some(json!({
                "commit": step.commit,
                "conflicted_files": step.conflicted_files,
            })),
        }];
    }

    if !has_reorder || prediction.status != "clean" {
        return Vec::new();
    }

    let mut blocks = Vec::new();
    let mut empty_commits = Vec::new();
    let mut previous_tree =
        read_tree_oid(scan, &scan.range.base_commit).expect("base tree is readable");
    for step in &prediction.steps {
        if step.merged_tree == previous_tree {
            empty_commits.push(step.commit.clone());
        }
        previous_tree = step.merged_tree.clone();
    }

    if let Some(final_tree) = prediction.final_tree.as_deref() {
        if Some(final_tree) != old_tree {
            blocks.push(history_edit::HistoryEditBlock {
                code: "reorder_changes_final_tree".to_string(),
                severity: "hard_block".to_string(),
                details: Some(json!({
                    "old_tree": old_tree,
                    "predicted_final_tree": final_tree,
                })),
            });
        }
    }

    if !empty_commits.is_empty() {
        blocks.push(history_edit::HistoryEditBlock {
            code: "reorder_creates_empty_commit".to_string(),
            severity: "hard_block".to_string(),
            details: Some(json!({
                "commits": empty_commits,
            })),
        });
    }

    blocks
}

fn build_plan(
    base: &str,
    scan: HistoryEditScan,
    validation: Option<InstructionValidation>,
    has_drop: bool,
    prediction: Option<HistoryEditPrediction>,
) -> HistoryEditPlan {
    let instructions_provided = validation.is_some();
    let instruction_blocks = validation
        .as_ref()
        .map(|validation| validation.blocks.clone())
        .unwrap_or_default();
    let program = validation.and_then(|validation| validation.program);
    let reorder = program
        .as_ref()
        .and_then(|program| reorder_advisory(&scan.range.commits, program));
    let has_reorder = reorder.is_some();
    let old_tree = has_reorder
        .then(|| read_tree_oid(&scan, &scan.head_commit).expect("HEAD tree is readable"));

    // 충돌 예측은 자동 해결 대상이 아니라 hard block이다. 증거(충돌 커밋과
    // per-file stage)를 같이 실어 에이전트가 어느 kept 커밋이 어디서
    // 부딪히는지 plan만으로 보게 한다.
    let prediction_blocks =
        prediction_blocks(&scan, prediction.as_ref(), has_reorder, old_tree.as_deref());

    let mut block_codes: HashSet<String> =
        scan.blocks.iter().map(|block| block.code.clone()).collect();
    block_codes.extend(instruction_blocks.iter().map(|block| block.code.clone()));
    block_codes.extend(prediction_blocks.iter().map(|block| block.code.clone()));
    let has_blocks = !block_codes.is_empty();

    let published_in_range = scan.range.commits.iter().any(|commit| commit.published);

    // Status is the single switch the rest of the plan reads from.
    // drop은 published 여부와 무관하게 항상 confirmation-gated다(C8-drop 계약:
    // content-deletion 의미론은 published와 직교한다).
    let status = if !instructions_provided {
        if scan.blocks.is_empty() {
            "survey"
        } else {
            "blocked"
        }
    } else if has_blocks {
        "blocked"
    } else if has_drop || published_in_range {
        "preview_only"
    } else {
        "executable"
    };

    let requires_confirmation_artifact = status == "preview_only";
    let executable = status == "executable" || status == "preview_only";

    let blocked_reasons = blocked_reasons(&scan, &instruction_blocks, &prediction_blocks);
    let suggested_super_git_command = suggested_command(status, base);
    let execution = HistoryEditExecution {
        status: status.to_string(),
        execute_supported: executable,
        requires_confirmation_artifact,
        raw_git_allowed: false,
        suggested_super_git_command,
        blocked_reasons,
    };

    let risk = ActionRisk {
        severity: if published_in_range || has_drop {
            "high"
        } else {
            "medium"
        }
        .to_string(),
        reversibility: "reversible_if_unchanged".to_string(),
        requires_human_confirmation: published_in_range || has_drop,
    };

    let branch = scan.branch.as_ref().map(|branch| HistoryEditPlanBranch {
        ref_name: branch.ref_name.clone(),
        short_name: branch.short_name.clone(),
        tip_commit: branch.tip_commit.clone(),
        checked_out_at: branch.checked_out_at.clone(),
        upstream: branch.upstream.clone(),
    });
    let branch_label = branch
        .as_ref()
        .map(|branch| branch.ref_name.clone())
        .unwrap_or_else(|| "HEAD".to_string());

    let published_commits = scan
        .range
        .commits
        .iter()
        .filter(|commit| commit.published)
        .map(|commit| commit.commit.clone())
        .collect::<Vec<_>>();

    let range = HistoryEditPlanRange {
        base_input: scan.range.base_input.clone(),
        base_commit: scan.range.base_commit.clone(),
        base_is_ancestor_of_head: scan.range.base_is_ancestor_of_head,
        order: scan.range.order.clone(),
        commit_count: scan.range.commit_count,
        commits: scan.range.commits.iter().map(plan_commit).collect(),
    };

    let instructions = program.as_ref().map(plan_instructions);
    let result_summary = program.as_ref().map(result_summary_view);

    // Survey plans hand back a ready-to-edit instruction document (every range
    // commit as `pick`) so the agent never reconstructs the schema by hand.
    let instructions_template = (status == "survey").then(|| HistoryEditInstructionsTemplate {
        schema_version: HISTORY_EDIT_INSTRUCTIONS_SCHEMA_VERSION.to_string(),
        action: ACTION_HISTORY_EDIT.to_string(),
        base: base.to_string(),
        items: scan
            .range
            .commits
            .iter()
            .map(|commit| HistoryEditPlanInstructionItem {
                commit: commit.commit.clone(),
                op: "pick".to_string(),
                message: None,
            })
            .collect(),
    });

    let confirmation = if requires_confirmation_artifact {
        let dropped_count = prediction
            .as_ref()
            .map(|prediction| prediction.dropped_commits.len())
            .unwrap_or(0);
        let mut reason_codes = Vec::new();
        if has_drop {
            reason_codes.push("tree_changing_drop".to_string());
        }
        if published_in_range {
            reason_codes.extend([
                "rewrites_published_commits".to_string(),
                "remote_branches_will_diverge".to_string(),
                "local_undo_does_not_unpublish".to_string(),
            ]);
        }
        // 한 plan에 phrase는 하나: drop이 있으면 가장 위험한 의미(drop)를
        // 이름 붙인 phrase가 published phrase를 대신한다(reason code는 둘 다).
        let (human_prompt, required_phrase) = if has_drop {
            (
                format!("Drop {dropped_count} commit(s) from {branch_label}?"),
                None,
            )
        } else {
            (
                format!("Rewrite published history on {branch_label}?"),
                None,
            )
        };
        Some(PreviewConfirmation {
            required_before_execute: true,
            reason_codes,
            human_prompt,
            required_phrase,
        })
    } else {
        None
    };

    let warnings = scan
        .warnings
        .iter()
        .map(|warning| HistoryEditPlanWarning {
            code: warning.code.clone(),
        })
        .collect();

    let preconditions = preconditions(
        &block_codes,
        instructions_provided,
        has_drop,
        has_reorder,
        prediction.as_ref(),
    );

    let mut plan = HistoryEditPlan {
        schema_version: HISTORY_EDIT_PLAN_SCHEMA_VERSION.to_string(),
        plan_id: String::new(),
        action: HistoryEditAction {
            kind: ACTION_HISTORY_EDIT.to_string(),
            options: HistoryEditOptions {
                base: base.to_string(),
            },
        },
        repository: HistoryEditPlanRepository {
            family_id: scan.repository.family_id,
            git_common_dir: scan.repository.git_common_dir,
            worktree_root: scan.repository.worktree_root,
        },
        branch,
        range,
        published_scan: HistoryEditPublishedScan {
            basis: scan.range.published_scan_basis.clone(),
            published_commits,
        },
        instructions,
        instructions_template,
        result_summary,
        reorder,
        prediction,
        preconditions,
        execution,
        risk,
        confirmation,
        warnings,
        effects: effects(status, program.as_ref(), &branch_label, has_reorder),
        limitations: limitations(has_drop, has_reorder),
        reference_commands: WorktreeReferenceCommands {
            semantics: "documentation_only".to_string(),
            never_execute_directly: true,
            commands: vec![vec![
                "git".to_string(),
                "rebase".to_string(),
                "-i".to_string(),
                base.to_string(),
            ]],
        },
        undo_strategy: undo_strategy(has_drop),
        undo_preview: undo_preview(has_drop, executable),
    };
    plan.plan_id = compute_history_edit_plan_id(&plan);
    if let Some(required_phrase) = confirmation_required_phrase(
        &plan,
        has_drop,
        plan.prediction.as_ref(),
        plan.branch.as_ref(),
    ) {
        if let Some(confirmation) = plan.confirmation.as_mut() {
            confirmation.required_phrase = Some(required_phrase);
        }
    }
    plan
}

fn confirmation_required_phrase(
    plan: &HistoryEditPlan,
    has_drop: bool,
    prediction: Option<&HistoryEditPrediction>,
    branch: Option<&HistoryEditPlanBranch>,
) -> Option<String> {
    let branch = branch?;
    plan.confirmation.as_ref()?;
    if has_drop {
        let dropped_count = prediction
            .map(|prediction| prediction.dropped_commits.len())
            .unwrap_or(0);
        Some(drop_confirmation_phrase(
            dropped_count,
            &branch.ref_name,
            &branch.tip_commit,
            &plan.plan_id,
        ))
    } else {
        Some(confirmation_phrase(
            &branch.ref_name,
            &branch.tip_commit,
            &plan.plan_id,
        ))
    }
}

fn plan_commit(commit: &HistoryEditCommit) -> HistoryEditPlanCommit {
    HistoryEditPlanCommit {
        commit: commit.commit.clone(),
        subject: commit.subject.clone(),
        message: commit.message.clone(),
        author_name: commit.author_name.clone(),
        author_email: commit.author_email.clone(),
        author_date: commit.author_date.clone(),
        published: commit.published,
        signed: commit.signed,
        is_merge: commit.is_merge,
    }
}

fn plan_instructions(program: &HistoryEditProgram) -> HistoryEditPlanInstructions {
    HistoryEditPlanInstructions {
        schema_version: HISTORY_EDIT_INSTRUCTIONS_SCHEMA_VERSION.to_string(),
        order: "oldest_first".to_string(),
        items: program
            .steps
            .iter()
            .map(|step| HistoryEditPlanInstructionItem {
                commit: step.commit.clone(),
                op: step.op.clone(),
                message: step.message.clone(),
            })
            .collect(),
    }
}

fn result_summary_view(program: &HistoryEditProgram) -> HistoryEditResultSummaryView {
    HistoryEditResultSummaryView {
        commits_before: program.summary.commits_before,
        commits_after: program.summary.commits_after,
        messages_changed: program.summary.messages_changed,
        commits_folded: program.summary.commits_folded,
        commits_dropped: program.summary.commits_dropped,
        final_tree_unchanged: program.summary.final_tree_unchanged,
    }
}

fn blocked_reasons(
    scan: &HistoryEditScan,
    instruction_blocks: &[history_edit::HistoryEditBlock],
    prediction_blocks: &[history_edit::HistoryEditBlock],
) -> Vec<HistoryEditBlockedReason> {
    scan.blocks
        .iter()
        .chain(instruction_blocks.iter())
        .chain(prediction_blocks.iter())
        .map(|block| HistoryEditBlockedReason {
            code: block.code.clone(),
            severity: block.severity.clone(),
            details: block.details.clone().unwrap_or_else(|| json!({})),
        })
        .collect()
}

fn suggested_command(status: &str, base: &str) -> Option<Vec<String>> {
    match status {
        "survey" => Some(vec![
            "super-git".to_string(),
            "preview".to_string(),
            "history-edit".to_string(),
            "--base".to_string(),
            base.to_string(),
            "--instructions".to_string(),
            "<instructions-file>".to_string(),
        ]),
        "executable" => Some(vec![
            "super-git".to_string(),
            "execute".to_string(),
            "--plan".to_string(),
            "<plan-file>".to_string(),
        ]),
        "preview_only" => Some(vec![
            "super-git".to_string(),
            "execute".to_string(),
            "--plan".to_string(),
            "<plan-file>".to_string(),
            "--confirmation".to_string(),
            "<confirmation-file>".to_string(),
        ]),
        _ => None,
    }
}

fn preconditions(
    block_codes: &HashSet<String>,
    instructions_provided: bool,
    has_drop: bool,
    has_reorder: bool,
    prediction: Option<&HistoryEditPrediction>,
) -> Vec<HistoryEditPrecondition> {
    let mut preconditions: Vec<HistoryEditPrecondition> = [
        ("head_attached_to_local_branch", &["head_detached"][..]),
        ("operation_none", &["operation_in_progress"]),
        ("no_conflicted_paths", &["conflicts_present"]),
        ("base_is_ancestor_of_head", &["base_not_ancestor_of_head"]),
        (
            "range_linear_without_merges",
            &["range_empty", "range_too_large", "merge_commit_in_range"],
        ),
        ("commit_signing_disabled", &["commit_signing_enabled"]),
        (
            "committer_identity_configured",
            &["committer_identity_missing"],
        ),
    ]
    .into_iter()
    .map(|(code, blocking_codes)| HistoryEditPrecondition {
        code: code.to_string(),
        status: blocked_or_passed(block_codes, blocking_codes).to_string(),
    })
    .collect();

    // instructions_match_range is meaningless until instructions are supplied,
    // so a survey reports it as not_applicable rather than a false pass.
    let instructions_status = if !instructions_provided {
        "not_applicable"
    } else if INSTRUCTION_BLOCK_CODES
        .iter()
        .any(|code| block_codes.contains(*code))
    {
        "blocked"
    } else {
        "passed"
    };
    preconditions.push(HistoryEditPrecondition {
        code: "instructions_match_range".to_string(),
        status: instructions_status.to_string(),
    });

    // replay 기반 op(drop/reorder)의 precondition. 상태값은 의도적으로
    // 비휘발적이다:
    // 워킹트리 청결 같은 휘발 상태를 여기 넣으면 plan_id가 흔들려서
    // "preview는 dirty, execute는 clean" 같은 정상 흐름이 깨진다.
    let replay_status = if !(has_drop || has_reorder) {
        "not_applicable"
    } else if block_codes.contains("predicted_conflict") {
        "blocked"
    } else if prediction.is_some_and(|prediction| prediction.status == "clean") {
        "passed"
    } else {
        // 다른 block 때문에 예측 자체를 돌리지 않았다.
        "not_applicable"
    };
    preconditions.push(HistoryEditPrecondition {
        code: "replay_predicts_no_conflicts".to_string(),
        status: replay_status.to_string(),
    });
    // tree-preserving op들과 달리 drop execute는 clean 워킹트리를 하드
    // 요구하고 실행 후 워킹트리를 새 tip으로 동기화한다(C8-drop 계약).
    // 시점은 execute다 — preview는 read-only라 dirty를 막지 않는다.
    preconditions.push(HistoryEditPrecondition {
        code: "working_tree_clean_required_at_execute".to_string(),
        status: if has_drop {
            "enforced_at_execute"
        } else {
            "not_applicable"
        }
        .to_string(),
    });
    preconditions
}

fn blocked_or_passed(block_codes: &HashSet<String>, blocking_codes: &[&str]) -> &'static str {
    if blocking_codes
        .iter()
        .any(|code| block_codes.contains(*code))
    {
        "blocked"
    } else {
        "passed"
    }
}

fn effects(
    status: &str,
    program: Option<&HistoryEditProgram>,
    branch_label: &str,
    has_reorder: bool,
) -> Vec<String> {
    match (status, program, has_reorder) {
        // tree-changing(drop) plan은 효과 설명도 달라야 한다: 트리가 보존된다는
        // 문장은 거짓이 되고, execute의 워킹트리 동기화가 새로 등장한다.
        ("executable" | "preview_only", Some(program), _)
            if program.summary.commits_dropped > 0 =>
        {
            let summary = &program.summary;
            vec![
                format!(
                    "Rewrite {} commits on {} into {} commits, removing {} commit's patch(es) from the final history.",
                    summary.commits_before,
                    branch_label,
                    summary.commits_after,
                    summary.commits_dropped
                ),
                "Change the final tree: the dropped changes disappear from the branch tip.".to_string(),
                "Preserve every author name, email, and date on kept commits.".to_string(),
                "Execute will require a clean working tree and will synchronize it to the new tip.".to_string(),
            ]
        }
        ("executable" | "preview_only", Some(program), true) => {
            let summary = &program.summary;
            vec![
                format!(
                    "Reorder {} commits on {}.",
                    summary.commits_before, branch_label
                ),
                "Preserve the final tree; working-tree files and the index do not change."
                    .to_string(),
                "Change commit object ids and intermediate history shape from the first reordered position.".to_string(),
            ]
        }
        ("executable" | "preview_only", Some(program), _) => {
            let summary = &program.summary;
            vec![
                format!(
                    "Rewrite {} commits on {} into {} commits.",
                    summary.commits_before, branch_label, summary.commits_after
                ),
                format!(
                    "Change {} commit message(s) and fold {} commit(s).",
                    summary.messages_changed, summary.commits_folded
                ),
                "Preserve every author name, email, and date.".to_string(),
                "Preserve the final tree; working-tree files and the index do not change."
                    .to_string(),
            ]
        }
        ("survey", _, _) => vec![
            "Survey only: no instructions were provided, so no write is planned.".to_string(),
            "range.commits is the template the instruction list must follow.".to_string(),
        ],
        _ => vec!["No write is allowed until blocked reasons are resolved.".to_string()],
    }
}

fn limitations(has_drop: bool, has_reorder: bool) -> Vec<String> {
    let mut limitations = vec![
        "Published detection only sees local remote-tracking refs from the last fetch.".to_string(),
        "Undo depends on the previous tip staying reachable in the local object store.".to_string(),
        "Rewritten commits do not preserve GPG/SSH signatures from the originals.".to_string(),
    ];
    if has_drop || has_reorder {
        limitations.push(
            "The replay prediction is commit-level and ignores the index and working tree."
                .to_string(),
        );
    }
    if has_drop {
        limitations.extend([
            "The dropped commits' objects may remain in the object database; drop changes what the branch points at, not what exists.".to_string(),
        ]);
    }
    if has_reorder {
        limitations.extend([
            "Reorder preserves the final tree only; intermediate commit trees and object ids may change."
                .to_string(),
        ]);
    }
    limitations
}

fn undo_strategy(has_drop: bool) -> HistoryEditUndoStrategy {
    HistoryEditUndoStrategy {
        // drop execute는 워킹트리를 동기화하므로 undo도 대칭으로 복원해야
        // 한다. 새 kind라 구버전 바이너리는 fail-closed로 거부한다.
        kind: if has_drop {
            "restore_branch_tip_and_worktree"
        } else {
            "restore_branch_tip_snapshot"
        }
        .to_string(),
        deletes_branch: false,
        deletes_history: false,
    }
}

fn undo_preview(has_drop: bool, executable: bool) -> HistoryEditUndoPreview {
    if has_drop {
        HistoryEditUndoPreview {
            kind: "restore_branch_tip_and_worktree".to_string(),
            available_after_execute: executable,
            limitations: vec![
                "Undo refuses if the branch tip moved after execute.".to_string(),
                "Undo requires the previous tip commit to still exist locally (reflog/gc window)."
                    .to_string(),
                "Undo requires a clean working tree, restores the branch pointer, and synchronizes the working tree back to the previous tip.".to_string(),
                "The dropped commits' objects may remain in the object database; undo neither depends on deleting them nor attempts to.".to_string(),
                "Undo does not un-publish anything that was pushed.".to_string(),
            ],
        }
    } else {
        HistoryEditUndoPreview {
            kind: "restore_branch_tip_snapshot".to_string(),
            available_after_execute: executable,
            limitations: vec![
                "Undo refuses if the branch tip moved after execute.".to_string(),
                "Undo requires the previous tip commit to still exist locally (reflog/gc window)."
                    .to_string(),
                "Undo restores the branch pointer only; it never touches working-tree files."
                    .to_string(),
                "Undo does not un-publish anything that was pushed.".to_string(),
            ],
        }
    }
}

/// The deterministic typed phrase a published-range history_edit confirmation
/// must carry. Shared by preview (which advertises it in the plan) and execute
/// (which enforces it), so the two can never drift.
pub fn confirmation_phrase(branch_ref: &str, tip_commit: &str, plan_id: &str) -> String {
    format!(
        "rewrite published history on {branch_ref} at {tip_commit} for plan {}",
        short_plan_id(plan_id)
    )
}

/// drop plan의 deterministic confirmation phrase (C8-drop 계약). published
/// phrase와 같은 anti-drift 패턴: preview가 광고하고 execute가 강제한다.
pub fn drop_confirmation_phrase(
    count: usize,
    branch_ref: &str,
    tip_commit: &str,
    plan_id: &str,
) -> String {
    format!(
        "drop {count} commit(s) from {branch_ref} at {tip_commit} for plan {}",
        short_plan_id(plan_id)
    )
}

fn short_plan_id(plan_id: &str) -> &str {
    let digest = plan_id.strip_prefix("sha256:").unwrap_or(plan_id);
    &digest[..digest.len().min(12)]
}

pub fn compute_history_edit_plan_id(plan: &HistoryEditPlan) -> String {
    let hash_input = HistoryEditPlanHashInput {
        schema_version: &plan.schema_version,
        action: &plan.action,
        repository: &plan.repository,
        branch: plan
            .branch
            .as_ref()
            .map(|branch| HistoryEditBranchHashInput {
                ref_name: &branch.ref_name,
                tip_commit: &branch.tip_commit,
                checked_out_at: branch.checked_out_at.to_string_lossy().into_owned(),
            }),
        range: HistoryEditRangeHashInput {
            base_input: &plan.range.base_input,
            base_commit: &plan.range.base_commit,
            base_is_ancestor_of_head: plan.range.base_is_ancestor_of_head,
            order: &plan.range.order,
            commit_count: plan.range.commit_count,
            commits: plan
                .range
                .commits
                .iter()
                .map(|commit| HistoryEditCommitHashInput {
                    commit: &commit.commit,
                    published: commit.published,
                    signed: commit.signed,
                })
                .collect(),
        },
        published_scan: &plan.published_scan,
        instructions: plan.instructions.as_ref(),
        result_summary: plan.result_summary.as_ref(),
        // 예측 증거는 advisory가 아니다: final_tree가 execute의 post-verify
        // 오라클이므로 통째로 plan_id에 바인딩한다(변조 = plan_id mismatch).
        prediction: plan.prediction.as_ref(),
        preconditions: &plan.preconditions,
        execution: HistoryEditExecutionHashInput {
            status: &plan.execution.status,
            execute_supported: plan.execution.execute_supported,
            requires_confirmation_artifact: plan.execution.requires_confirmation_artifact,
            raw_git_allowed: plan.execution.raw_git_allowed,
            blocked_reasons: &plan.execution.blocked_reasons,
        },
        risk: &plan.risk,
        confirmation: plan.confirmation.as_ref().map(|confirmation| {
            HistoryEditConfirmationHashInput {
                required_before_execute: confirmation.required_before_execute,
                reason_codes: &confirmation.reason_codes,
            }
        }),
        undo_strategy: &plan.undo_strategy,
    };
    // hash domain은 schema_version과 같은 버전을 따라간다: drop의 prediction이
    // projection에 들어오며 v0.4 → v0.5로 바뀌었다.
    sha256_with_domain(b"super-git-plan-v0.5\n", &hash_input)
}

fn sha256_with_domain<T: Serialize>(domain: &[u8], value: &T) -> String {
    let canonical = canonical_json_bytes(value);
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(canonical);
    format_digest(hasher.finalize().as_slice())
}

fn canonical_json_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    let value = serde_json::to_value(value).expect("hash input serializes to JSON");
    serde_json::to_vec(&sort_json_value(value)).expect("sorted JSON serializes")
}

fn sort_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(sort_json_value).collect())
        }
        serde_json::Value::Object(map) => {
            let mut entries = map.into_iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));

            let mut sorted = serde_json::Map::new();
            for (key, value) in entries {
                sorted.insert(key, sort_json_value(value));
            }
            serde_json::Value::Object(sorted)
        }
        value => value,
    }
}

fn format_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity("sha256:".len() + bytes.len() * 2);
    output.push_str("sha256:");
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

#[derive(Serialize)]
struct HistoryEditPlanHashInput<'a> {
    schema_version: &'a str,
    action: &'a HistoryEditAction,
    repository: &'a HistoryEditPlanRepository,
    branch: Option<HistoryEditBranchHashInput<'a>>,
    range: HistoryEditRangeHashInput<'a>,
    published_scan: &'a HistoryEditPublishedScan,
    instructions: Option<&'a HistoryEditPlanInstructions>,
    result_summary: Option<&'a HistoryEditResultSummaryView>,
    prediction: Option<&'a HistoryEditPrediction>,
    preconditions: &'a [HistoryEditPrecondition],
    execution: HistoryEditExecutionHashInput<'a>,
    risk: &'a ActionRisk,
    confirmation: Option<HistoryEditConfirmationHashInput<'a>>,
    undo_strategy: &'a HistoryEditUndoStrategy,
}

#[derive(Serialize)]
struct HistoryEditBranchHashInput<'a> {
    ref_name: &'a str,
    tip_commit: &'a str,
    checked_out_at: String,
}

#[derive(Serialize)]
struct HistoryEditRangeHashInput<'a> {
    base_input: &'a str,
    base_commit: &'a str,
    base_is_ancestor_of_head: bool,
    order: &'a str,
    commit_count: usize,
    commits: Vec<HistoryEditCommitHashInput<'a>>,
}

#[derive(Serialize)]
struct HistoryEditCommitHashInput<'a> {
    commit: &'a str,
    published: bool,
    signed: bool,
}

#[derive(Serialize)]
struct HistoryEditExecutionHashInput<'a> {
    status: &'a str,
    execute_supported: bool,
    requires_confirmation_artifact: bool,
    raw_git_allowed: bool,
    blocked_reasons: &'a [HistoryEditBlockedReason],
}

#[derive(Serialize)]
struct HistoryEditConfirmationHashInput<'a> {
    required_before_execute: bool,
    reason_codes: &'a [String],
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::{Command, Output};

    use super::{compute_history_edit_plan_id, preview_history_edit};
    use crate::model::HistoryEditPlan;

    fn run_git(dir: &Path, args: &[&str]) -> Output {
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
        let output = run_git(dir, args);
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(dir: &Path, args: &[&str]) -> String {
        let output = run_git(dir, args);
        assert!(output.status.success(), "git {args:?} failed");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_repo(repo: &Path) {
        std::fs::create_dir_all(repo).expect("create repo");
        git(repo, &["init", "-q", "-b", "main"]);
        git(repo, &["config", "user.name", "test"]);
        git(repo, &["config", "user.email", "test@example.com"]);
        git(repo, &["config", "commit.gpgsign", "false"]);
        commit_file(repo, "README.md", "hello\n", "initial");
    }

    fn commit_file(repo: &Path, file: &str, content: &str, message: &str) {
        std::fs::write(repo.join(file), content).expect("write file");
        git(repo, &["add", file]);
        git(repo, &["commit", "-q", "-m", message]);
    }

    fn feature_repo(temp: &Path) -> (std::path::PathBuf, Vec<String>) {
        let repo = temp.join("repo");
        init_repo(&repo);
        git(&repo, &["checkout", "-q", "-b", "feature/login"]);
        commit_file(&repo, "a.txt", "a\n", "feat: add form");
        commit_file(&repo, "b.txt", "b\n", "fix typo");
        commit_file(&repo, "c.txt", "c\n", "wip");
        let oids = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
            .lines()
            .map(str::to_string)
            .collect();
        (repo, oids)
    }

    fn instructions(items: &str) -> Vec<u8> {
        format!(
            r#"{{"schema_version":"super-git.instructions.v0.1","action":"history_edit","base":"main","items":{items}}}"#
        )
        .into_bytes()
    }

    #[test]
    fn survey_plan_has_null_instructions_and_survey_status() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, _) = feature_repo(temp.path());

        let plan = preview_history_edit(&repo, "main", None).expect("survey plan");

        assert_eq!(plan.schema_version, "super-git.plan.v0.5");
        assert!(plan.plan_id.starts_with("sha256:"));
        assert_eq!(plan.execution.status, "survey");
        assert!(!plan.execution.execute_supported);
        assert!(plan.instructions.is_none());
        assert!(plan.result_summary.is_none());
        assert!(plan.confirmation.is_none());
        assert_eq!(plan.range.commit_count, 3);
        assert_eq!(plan.risk.severity, "medium");
        // not_applicable until instructions arrive.
        let instructions_precondition = plan
            .preconditions
            .iter()
            .find(|precondition| precondition.code == "instructions_match_range")
            .expect("precondition");
        assert_eq!(instructions_precondition.status, "not_applicable");
    }

    #[test]
    fn executable_plan_carries_resolved_instructions_and_summary() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = feature_repo(temp.path());
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"fix: typo"}},{{"commit":"{}","op":"fixup"}}]"#,
            oids[0], oids[1], oids[2]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        assert_eq!(plan.execution.status, "executable");
        assert!(plan.execution.execute_supported);
        assert!(!plan.execution.requires_confirmation_artifact);
        let instructions = plan.instructions.expect("instructions");
        assert_eq!(instructions.items.len(), 3);
        assert_eq!(
            instructions.items[1].message.as_deref(),
            Some("fix: typo\n")
        );
        let summary = plan.result_summary.expect("summary");
        assert_eq!(summary.commits_after, 2);
        assert!(summary.final_tree_unchanged);
        assert!(plan.undo_preview.available_after_execute);
    }

    #[test]
    fn published_range_requires_confirmation_artifact() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = feature_repo(temp.path());
        git(
            &repo,
            &["update-ref", "refs/remotes/origin/feature/login", &oids[2]],
        );
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"fix: typo"}},{{"commit":"{}","op":"pick"}}]"#,
            oids[0], oids[1], oids[2]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        assert_eq!(plan.execution.status, "preview_only");
        assert!(plan.execution.requires_confirmation_artifact);
        assert_eq!(plan.risk.severity, "high");
        assert!(plan.risk.requires_human_confirmation);
        let confirmation = plan.confirmation.expect("confirmation");
        assert!(confirmation
            .reason_codes
            .contains(&"rewrites_published_commits".to_string()));
        assert_eq!(plan.published_scan.published_commits.len(), 3);
    }

    #[test]
    fn invalid_instructions_block_and_drop_program() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = feature_repo(temp.path());
        // Missing the last commit makes the list incomplete.
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"m"}}]"#,
            oids[0], oids[1]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        assert_eq!(plan.execution.status, "blocked");
        assert!(plan.instructions.is_none());
        assert!(plan.result_summary.is_none());
        let codes: Vec<&str> = plan
            .execution
            .blocked_reasons
            .iter()
            .map(|reason| reason.code.as_str())
            .collect();
        assert!(codes.contains(&"instructions_incomplete"));
        let instructions_precondition = plan
            .preconditions
            .iter()
            .find(|precondition| precondition.code == "instructions_match_range")
            .expect("precondition");
        assert_eq!(instructions_precondition.status, "blocked");
    }

    #[test]
    fn scan_block_keeps_program_but_blocks_status() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = feature_repo(temp.path());
        // Signing is a scan-level block, but the instruction list itself is valid,
        // so the resolved program stays visible while status is blocked.
        git(&repo, &["config", "commit.gpgsign", "true"]);
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"m"}},{{"commit":"{}","op":"pick"}}]"#,
            oids[0], oids[1], oids[2]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        assert_eq!(plan.execution.status, "blocked");
        assert!(
            plan.instructions.is_some(),
            "a valid instruction list stays visible even when a scan block applies"
        );
        let codes: Vec<&str> = plan
            .execution
            .blocked_reasons
            .iter()
            .map(|reason| reason.code.as_str())
            .collect();
        assert!(codes.contains(&"commit_signing_enabled"));
    }

    #[test]
    fn detached_head_blocks_survey_with_null_branch() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, _) = feature_repo(temp.path());
        git(&repo, &["checkout", "-q", "--detach"]);

        let plan = preview_history_edit(&repo, "main", None).expect("plan");

        assert_eq!(plan.execution.status, "blocked");
        assert!(plan.branch.is_none());
        let codes: Vec<&str> = plan
            .execution
            .blocked_reasons
            .iter()
            .map(|reason| reason.code.as_str())
            .collect();
        assert!(codes.contains(&"head_detached"));
    }

    // ---- C8-drop-B: drop preview ----

    /// f.txt에 의존적 편집을 가진 체인: c2와 c3가 같은 줄을 잇달아 바꾼다.
    /// c2를 drop하면 c3 replay가 충돌한다.
    fn dependent_repo(temp: &Path) -> (std::path::PathBuf, Vec<String>) {
        let repo = temp.join("repo");
        init_repo(&repo);
        std::fs::write(repo.join("f.txt"), "a\nb\nc\n").expect("write");
        git(&repo, &["add", "f.txt"]);
        git(&repo, &["commit", "-q", "-m", "add f"]);
        git(&repo, &["checkout", "-q", "-b", "feature/dep"]);
        commit_file(&repo, "a.txt", "a\n", "c1 unrelated");
        std::fs::write(repo.join("f.txt"), "X\nb\nc\n").expect("write");
        git(&repo, &["commit", "-q", "-am", "c2 line1 X"]);
        std::fs::write(repo.join("f.txt"), "Y\nb\nc\n").expect("write");
        git(&repo, &["commit", "-q", "-am", "c3 line1 Y"]);
        let oids = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
            .lines()
            .map(str::to_string)
            .collect();
        (repo, oids)
    }

    /// f.txt: 1 -> X -> 1. Swapping the pair produces two clean replay steps,
    /// but the final tree becomes X and the first replayed step is empty.
    fn revert_pair_repo(temp: &Path) -> (std::path::PathBuf, Vec<String>) {
        let repo = temp.join("repo");
        init_repo(&repo);
        commit_file(&repo, "f.txt", "1\n", "base f");
        git(&repo, &["checkout", "-q", "-b", "feature/revert-pair"]);
        commit_file(&repo, "f.txt", "X\n", "set X");
        commit_file(&repo, "f.txt", "1\n", "restore 1");
        let oids = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
            .lines()
            .map(str::to_string)
            .collect();
        (repo, oids)
    }

    #[test]
    fn drop_clean_is_confirmation_gated_preview_only() {
        let temp = tempfile::tempdir().expect("temp");
        // feature_repo의 세 커밋은 서로 다른 파일을 만들므로 어떤 drop도 clean이다.
        let (repo, oids) = feature_repo(temp.path());
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
            oids[0], oids[1], oids[2]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        // drop은 published 여부와 무관하게 항상 confirmation-gated.
        assert_eq!(plan.execution.status, "preview_only");
        assert!(plan.execution.requires_confirmation_artifact);
        // C8-drop-C부터 execute가 지원된다: confirmation을 동반한 execute 안내.
        assert!(plan.execution.execute_supported);
        let suggested = plan
            .execution
            .suggested_super_git_command
            .expect("suggested command");
        assert!(suggested.contains(&"--confirmation".to_string()));
        assert_eq!(plan.risk.severity, "high");
        assert!(plan.risk.requires_human_confirmation);

        let confirmation = plan.confirmation.expect("confirmation");
        assert_eq!(confirmation.reason_codes, vec!["tree_changing_drop"]);
        let tip = git_stdout(&repo, &["rev-parse", "HEAD"]);
        let plan_short = plan
            .plan_id
            .strip_prefix("sha256:")
            .unwrap_or(&plan.plan_id);
        let plan_short = &plan_short[..plan_short.len().min(12)];
        assert_eq!(
            confirmation.required_phrase.as_deref(),
            Some(
                format!(
                    "drop 1 commit(s) from refs/heads/feature/login at {tip} for plan {plan_short}"
                )
                .as_str()
            )
        );

        // 예측 증거: clean, kept 2 step, final_tree는 드랍된 파일이 없는 트리.
        let prediction = plan.prediction.expect("prediction");
        assert_eq!(prediction.kind, "kept_commit_replay");
        assert_eq!(prediction.status, "clean");
        assert_eq!(prediction.dropped_commits, vec![oids[1].clone()]);
        assert_eq!(prediction.steps.len(), 2);
        // 두 번째 kept 커밋(c.txt)의 3-way base는 원래 parent(드랍된 b 커밋)다.
        assert_eq!(prediction.steps[1].parent, oids[1]);
        let final_tree = prediction.final_tree.expect("final tree");
        let listing = git_stdout(&repo, &["ls-tree", "--name-only", &final_tree]);
        assert!(listing.contains("c.txt"));
        assert!(
            !listing.contains("b.txt"),
            "dropped patch must vanish: {listing}"
        );

        let summary = plan.result_summary.expect("summary");
        assert_eq!(summary.commits_dropped, 1);
        assert_eq!(summary.commits_after, 2);
        assert!(!summary.final_tree_unchanged);

        // undo는 워킹트리 동기화 포함 kind를 광고한다(구버전 fail-closed,
        // C8-drop-D부터 구현됨).
        assert_eq!(plan.undo_strategy.kind, "restore_branch_tip_and_worktree");
        assert_eq!(plan.undo_preview.kind, "restore_branch_tip_and_worktree");
        assert!(plan.undo_preview.available_after_execute);

        let precondition_status = |code: &str| {
            plan.preconditions
                .iter()
                .find(|precondition| precondition.code == code)
                .map(|precondition| precondition.status.clone())
                .unwrap_or_else(|| panic!("missing precondition {code}"))
        };
        assert_eq!(
            precondition_status("replay_predicts_no_conflicts"),
            "passed"
        );
        assert_eq!(
            precondition_status("working_tree_clean_required_at_execute"),
            "enforced_at_execute"
        );
    }

    #[test]
    fn drop_predicted_conflict_is_blocked_with_step_evidence() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = dependent_repo(temp.path());
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
            oids[0], oids[1], oids[2]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        assert_eq!(plan.execution.status, "blocked");
        assert!(!plan.execution.execute_supported);
        assert!(plan.confirmation.is_none());
        let reason = plan
            .execution
            .blocked_reasons
            .iter()
            .find(|reason| reason.code == "predicted_conflict")
            .expect("predicted_conflict block");
        assert_eq!(reason.details["commit"], oids[2]);
        assert_eq!(reason.details["conflicted_files"][0]["path"], "f.txt");

        let prediction = plan.prediction.expect("prediction evidence stays visible");
        assert_eq!(prediction.status, "conflicted");
        assert!(prediction.final_tree.is_none());
        // 충돌 step의 per-file stage shape가 그대로 실린다.
        let conflicting = prediction.steps.last().expect("conflicting step");
        assert_eq!(conflicting.status, "conflicted");
        assert!(!conflicting.conflicted_files.is_empty());
        let precondition = plan
            .preconditions
            .iter()
            .find(|precondition| precondition.code == "replay_predicts_no_conflicts")
            .expect("precondition");
        assert_eq!(precondition.status, "blocked");
    }

    #[test]
    fn drop_with_fold_is_blocked_without_prediction() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = feature_repo(temp.path());
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"squash","message":"m"}}]"#,
            oids[0], oids[1], oids[2]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        assert_eq!(plan.execution.status, "blocked");
        let codes: Vec<&str> = plan
            .execution
            .blocked_reasons
            .iter()
            .map(|reason| reason.code.as_str())
            .collect();
        assert!(codes.contains(&"drop_with_fold_unsupported"));
        assert!(plan.prediction.is_none());
    }

    #[test]
    fn drop_everything_predicts_the_base_tree() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = feature_repo(temp.path());
        let items = format!(
            r#"[{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"drop"}}]"#,
            oids[0], oids[1], oids[2]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        assert_eq!(plan.execution.status, "preview_only");
        let prediction = plan.prediction.expect("prediction");
        assert_eq!(prediction.status, "clean");
        assert!(prediction.steps.is_empty());
        // 전체 drop: 예측 최종 트리는 base의 트리 그대로(범위 abandon 의미).
        let base_tree = git_stdout(&repo, &["rev-parse", "main^{tree}"]);
        assert_eq!(prediction.final_tree.as_deref(), Some(base_tree.as_str()));
        let summary = plan.result_summary.expect("summary");
        assert_eq!(summary.commits_after, 0);
        assert_eq!(summary.commits_dropped, 3);
        let confirmation = plan.confirmation.expect("confirmation");
        assert!(confirmation
            .required_phrase
            .expect("phrase")
            .starts_with("drop 3 commit(s) from "));
    }

    #[test]
    fn drop_preview_works_on_dirty_tree_and_marks_execute_precondition() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = feature_repo(temp.path());
        std::fs::write(repo.join("a.txt"), "dirty edit\n").expect("write");
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
            oids[0], oids[1], oids[2]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        // preview는 read-only라 dirty를 막지 않는다. execute 요구만 표시한다.
        assert_eq!(plan.execution.status, "preview_only");
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.code == "working_tree_dirty"));
        let precondition = plan
            .preconditions
            .iter()
            .find(|precondition| precondition.code == "working_tree_clean_required_at_execute")
            .expect("precondition");
        assert_eq!(precondition.status, "enforced_at_execute");
    }

    #[test]
    fn drop_on_published_range_names_both_reasons_with_the_drop_phrase() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = feature_repo(temp.path());
        git(
            &repo,
            &["update-ref", "refs/remotes/origin/feature/login", &oids[2]],
        );
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
            oids[0], oids[1], oids[2]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        assert_eq!(plan.execution.status, "preview_only");
        let confirmation = plan.confirmation.expect("confirmation");
        assert!(confirmation
            .reason_codes
            .contains(&"tree_changing_drop".to_string()));
        assert!(confirmation
            .reason_codes
            .contains(&"rewrites_published_commits".to_string()));
        // phrase는 하나: 가장 위험한 의미(drop)가 이름을 가진다.
        assert!(confirmation
            .required_phrase
            .expect("phrase")
            .starts_with("drop 1 commit(s) from "));
    }

    #[test]
    fn drop_prediction_evidence_is_plan_id_bound() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = feature_repo(temp.path());
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
            oids[0], oids[1], oids[2]
        );
        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");
        let original = plan.plan_id.clone();

        // final_tree는 execute의 post-verify 오라클: 변조는 plan_id를 바꿔야 한다.
        let mut tampered: HistoryEditPlan = plan.clone();
        tampered.prediction.as_mut().unwrap().final_tree =
            Some("0000000000000000000000000000000000000000".to_string());
        assert_ne!(original, compute_history_edit_plan_id(&tampered));

        let mut tampered = plan.clone();
        tampered.prediction.as_mut().unwrap().steps[0].merged_tree =
            "0000000000000000000000000000000000000000".to_string();
        assert_ne!(original, compute_history_edit_plan_id(&tampered));

        let mut tampered = plan;
        tampered
            .prediction
            .as_mut()
            .unwrap()
            .dropped_commits
            .clear();
        assert_ne!(original, compute_history_edit_plan_id(&tampered));
    }

    // ---- C8-reorder-B: reorder preview ----

    #[test]
    fn clean_reorder_is_executable_after_execute_lands() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = feature_repo(temp.path());
        // feature_repo's commits touch independent files, so swapping c1/c2 is a
        // clean tree-preserving reorder.
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}}]"#,
            oids[1], oids[0], oids[2]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        assert_eq!(plan.execution.status, "executable");
        assert!(plan.execution.execute_supported);
        assert!(!plan.execution.requires_confirmation_artifact);
        assert!(plan.confirmation.is_none());
        assert!(plan.execution.blocked_reasons.is_empty());
        assert!(plan.instructions.is_some());
        assert!(plan.result_summary.is_some());
        let prediction = plan.prediction.expect("prediction");
        assert_eq!(prediction.kind, "reordered_commit_replay");
        assert_eq!(prediction.status, "clean");
        assert!(prediction.dropped_commits.is_empty());
        assert!(prediction.final_tree.is_some());
        assert_eq!(prediction.steps.len(), 3);
        assert_eq!(plan.undo_strategy.kind, "restore_branch_tip_snapshot");
        assert!(plan.undo_preview.available_after_execute);
    }

    #[test]
    fn published_reorder_requires_the_standard_history_edit_confirmation() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = feature_repo(temp.path());
        git(
            &repo,
            &["update-ref", "refs/remotes/origin/feature/login", &oids[2]],
        );
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}}]"#,
            oids[1], oids[0], oids[2]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        assert_eq!(plan.execution.status, "preview_only");
        assert!(plan.execution.execute_supported);
        assert!(plan.execution.requires_confirmation_artifact);
        assert_eq!(plan.risk.severity, "high");
        assert!(plan.risk.requires_human_confirmation);
        assert!(plan.execution.blocked_reasons.is_empty());
        let confirmation = plan.confirmation.expect("confirmation");
        assert!(confirmation
            .reason_codes
            .contains(&"rewrites_published_commits".to_string()));
        let tip = git_stdout(&repo, &["rev-parse", "HEAD"]);
        let plan_short = plan
            .plan_id
            .strip_prefix("sha256:")
            .unwrap_or(&plan.plan_id);
        let plan_short = &plan_short[..plan_short.len().min(12)];
        assert_eq!(
            confirmation.required_phrase.as_deref(),
            Some(
                format!(
                    "rewrite published history on refs/heads/feature/login at {tip} for plan {plan_short}"
                )
                .as_str()
            )
        );
    }

    #[test]
    fn reorder_that_changes_final_tree_and_creates_empty_step_is_blocked() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = revert_pair_repo(temp.path());
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}}]"#,
            oids[1], oids[0]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        assert_eq!(plan.execution.status, "blocked");
        assert!(!plan.execution.execute_supported);
        let codes: Vec<&str> = plan
            .execution
            .blocked_reasons
            .iter()
            .map(|reason| reason.code.as_str())
            .collect();
        assert!(codes.contains(&"reorder_changes_final_tree"));
        assert!(codes.contains(&"reorder_creates_empty_commit"));
        assert!(!codes.contains(&"reorder_execute_unsupported"));

        let old_tree = git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]);
        let prediction = plan.prediction.expect("prediction");
        assert_eq!(prediction.kind, "reordered_commit_replay");
        assert_eq!(prediction.status, "clean");
        assert_ne!(prediction.final_tree.as_deref(), Some(old_tree.as_str()));
        let empty_step = prediction.steps.first().expect("first step");
        let base_tree = git_stdout(&repo, &["rev-parse", "main^{tree}"]);
        assert_eq!(empty_step.commit, oids[1]);
        assert_eq!(empty_step.merged_tree, base_tree);
    }

    #[test]
    fn reorder_predicted_conflict_is_blocked_with_step_evidence() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = dependent_repo(temp.path());
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}}]"#,
            oids[0], oids[2], oids[1]
        );

        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");

        assert_eq!(plan.execution.status, "blocked");
        assert!(!plan.execution.execute_supported);
        let reason = plan
            .execution
            .blocked_reasons
            .iter()
            .find(|reason| reason.code == "predicted_conflict")
            .expect("predicted_conflict block");
        assert_eq!(reason.details["commit"], oids[2]);
        assert_eq!(reason.details["conflicted_files"][0]["path"], "f.txt");

        let prediction = plan.prediction.expect("prediction evidence stays visible");
        assert_eq!(prediction.kind, "reordered_commit_replay");
        assert_eq!(prediction.status, "conflicted");
        assert!(prediction.final_tree.is_none());
        assert!(!plan
            .execution
            .blocked_reasons
            .iter()
            .any(|reason| reason.code == "reorder_execute_unsupported"));
    }

    #[test]
    fn reorder_advisory_is_not_plan_id_bound() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = feature_repo(temp.path());
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}}]"#,
            oids[1], oids[0], oids[2]
        );
        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");
        let original = plan.plan_id.clone();

        let reorder = plan.reorder.as_ref().expect("reorder advisory");
        assert_eq!(reorder.commits_reordered, 2);
        assert_eq!(reorder.old_order, oids);

        let mut tampered = plan.clone();
        tampered.reorder.as_mut().unwrap().commits_reordered = 0;
        tampered.reorder.as_mut().unwrap().old_order.clear();
        assert_eq!(original, compute_history_edit_plan_id(&tampered));

        let mut tampered = plan;
        tampered.instructions.as_mut().unwrap().items.swap(0, 1);
        assert_ne!(original, compute_history_edit_plan_id(&tampered));
    }

    #[test]
    fn plan_id_ignores_advisory_fields() {
        let temp = tempfile::tempdir().expect("temp");
        let (repo, oids) = feature_repo(temp.path());
        let items = format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"m"}},{{"commit":"{}","op":"fixup"}}]"#,
            oids[0], oids[1], oids[2]
        );
        let plan = preview_history_edit(&repo, "main", Some(&instructions(&items))).expect("plan");
        let first = plan.plan_id.clone();

        let mut tampered: HistoryEditPlan = plan;
        tampered.effects = vec!["different prose".to_string()];
        tampered.limitations = vec!["different limitation".to_string()];
        tampered.reference_commands.commands = vec![vec!["git".to_string(), "status".to_string()]];
        tampered.execution.suggested_super_git_command = Some(vec!["different".to_string()]);
        // Author/subject prose is excluded; only object ids bind the range.
        tampered.range.commits[0].subject = "rewritten subject".to_string();
        tampered.range.commits[0].author_name = "Someone Else".to_string();
        tampered.warnings.clear();

        let second = compute_history_edit_plan_id(&tampered);
        assert_eq!(first, second);

        // A real change (an instruction message) must change the id.
        tampered.instructions.as_mut().unwrap().items[1].message = Some("changed\n".to_string());
        let third = compute_history_edit_plan_id(&tampered);
        assert_ne!(first, third);
    }
}
