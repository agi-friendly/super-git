use std::path::Path;

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::git::worktree_remove;
use crate::model::{
    ActionRisk, DestructivePreviewExecution, PreviewConfirmation, RecoveryHint,
    UnavailableUndoStrategy, WorktreeBlockedReason, WorktreeReferenceCommands,
    WorktreeRemoveAction, WorktreeRemoveOptions, WorktreeRemovePlan, WorktreeRemovePrecondition,
    WorktreeRemoveRepository, WorktreeRemoveTarget, WorktreeRemoveTargetState,
    WorktreeRemoveWorkingTree, DESTRUCTIVE_PREVIEW_PLAN_SCHEMA_VERSION,
};
use crate::Result;

const ACTION_WORKTREE_REMOVE: &str = "worktree_remove";

pub fn preview_worktree_remove(
    current_path: &Path,
    exact_target_path: &Path,
) -> Result<WorktreeRemovePlan> {
    let scan = worktree_remove::scan_worktree_remove_target(current_path, exact_target_path)?;
    let blocked_reasons = scan
        .blocks
        .iter()
        .map(|block| WorktreeBlockedReason {
            code: block.code.clone(),
            severity: block.severity.clone(),
            details: json!({}),
        })
        .collect::<Vec<_>>();
    let target_path = scan.target.worktree_list_path.clone();
    let status = scan.execution_status.clone();
    let future_execute_eligibility = if status == "preview_only" {
        "needs_human_confirmation"
    } else {
        "blocked"
    };
    let target_branch = scan.target.branch.clone();

    let mut plan = WorktreeRemovePlan {
        schema_version: DESTRUCTIVE_PREVIEW_PLAN_SCHEMA_VERSION.to_string(),
        plan_id: String::new(),
        action: WorktreeRemoveAction {
            kind: ACTION_WORKTREE_REMOVE.to_string(),
            options: WorktreeRemoveOptions {
                worktree: exact_target_path.to_path_buf(),
            },
        },
        repository: WorktreeRemoveRepository {
            family_id: scan.repository.family_id,
            git_common_dir: scan.repository.git_common_dir,
            main_worktree: scan.repository.main_worktree,
            selected_from: scan.repository.selected_from,
        },
        target: WorktreeRemoveTarget {
            input_path: scan.target.input_path,
            canonical_path: scan.target.canonical_path,
            worktree_list_path: scan.target.worktree_list_path,
            kind: scan.target.kind,
            worktree_git_dir: scan.target.worktree_git_dir,
            git_common_dir: scan.target.git_common_dir,
            head: scan.target.head,
            branch: scan.target.branch,
            detached: scan.target.detached,
            locked: scan.target.locked,
            prunable: scan.target.prunable,
            is_current_worktree: scan.target.is_current_worktree,
            has_submodules: scan.target.has_submodules,
        },
        target_state: WorktreeRemoveTargetState {
            operation: scan.target.operation,
            working_tree: WorktreeRemoveWorkingTree {
                clean: scan.target.working_tree.clean,
                staged: scan.target.working_tree.staged,
                unstaged: scan.target.working_tree.unstaged,
                untracked: scan.target.working_tree.untracked,
                ignored: scan.target.working_tree.ignored,
                conflict_count: scan.target.working_tree.conflict_count,
                conflicts: scan.target.working_tree.conflicts,
            },
        },
        preconditions: preconditions(&blocked_reasons),
        execution: DestructivePreviewExecution {
            status,
            execute_supported: future_execute_eligibility == "needs_human_confirmation",
            future_execute_eligibility: future_execute_eligibility.to_string(),
            raw_git_allowed: false,
            suggested_super_git_command: None,
            blocked_reasons,
        },
        risk: ActionRisk {
            severity: "high".to_string(),
            reversibility: "not_automatically_reversible".to_string(),
            requires_human_confirmation: true,
        },
        confirmation: PreviewConfirmation {
            required_before_execute: true,
            reason_codes: vec![
                "deletes_worktree_directory".to_string(),
                "git_worktree_metadata_changes".to_string(),
                "no_automatic_undo".to_string(),
            ],
            human_prompt: format!("Remove linked worktree at {}?", target_path.display()),
        },
        effects: effects_for(future_execute_eligibility, &target_path),
        limitations: vec![
            "Preview cannot detect editors, terminals, development servers, or file watchers using the target path.".to_string(),
            "`git worktree remove` deletes ignored files; an ignored file created between the final clean check and the removal may be lost.".to_string(),
        ],
        reference_commands: WorktreeReferenceCommands {
            semantics: "documentation_only".to_string(),
            never_execute_directly: true,
            commands: vec![vec![
                "git".to_string(),
                "worktree".to_string(),
                "remove".to_string(),
                target_path.display().to_string(),
            ]],
        },
        undo_strategy: UnavailableUndoStrategy {
            kind: "not_available".to_string(),
            reason: "Existing worktree removal is destructive and cannot restore untracked or ignored files.".to_string(),
        },
        recovery_hints: recovery_hints(&target_path, target_branch.as_deref()),
    };
    plan.plan_id = compute_worktree_remove_plan_id(&plan)?;
    Ok(plan)
}

fn preconditions(blocked_reasons: &[WorktreeBlockedReason]) -> Vec<WorktreeRemovePrecondition> {
    [
        (
            "target_is_linked_worktree",
            &["target_not_linked_worktree"][..],
        ),
        (
            "target_not_current_worktree",
            &["target_is_current_worktree"],
        ),
        ("target_not_detached", &["target_detached"]),
        ("target_not_locked", &["target_locked"]),
        ("target_not_prunable", &["target_prunable"]),
        ("target_family_matches", &["target_family_mismatch"]),
        (
            "target_has_no_in_progress_operation",
            &["operation_in_progress"],
        ),
        (
            "target_clean_including_ignored",
            &[
                "target_has_conflicts",
                "target_has_staged_changes",
                "target_has_unstaged_changes",
                "target_has_untracked_files",
                "target_has_ignored_files",
            ],
        ),
        ("target_has_no_submodules", &["target_has_submodules"]),
    ]
    .into_iter()
    .map(|(code, blocking_codes)| WorktreeRemovePrecondition {
        code: code.to_string(),
        status: if contains_any_block(blocked_reasons, blocking_codes) {
            "blocked"
        } else {
            "passed"
        }
        .to_string(),
    })
    .collect()
}

fn contains_any_block(blocked_reasons: &[WorktreeBlockedReason], codes: &[&str]) -> bool {
    blocked_reasons
        .iter()
        .any(|reason| codes.contains(&reason.code.as_str()))
}

fn effects_for(future_execute_eligibility: &str, target_path: &Path) -> Vec<String> {
    if future_execute_eligibility == "blocked" {
        return vec!["No write is allowed until blocked reasons are resolved.".to_string()];
    }

    vec![
        format!(
            "Delete the linked worktree directory at {}.",
            target_path.display()
        ),
        "Remove the linked worktree entry from Git worktree metadata.".to_string(),
        "Preserve branch refs, remote refs, commits, and history.".to_string(),
    ]
}

fn recovery_hints(target_path: &Path, branch: Option<&str>) -> Vec<RecoveryHint> {
    let ref_name = branch.unwrap_or("<branch-or-commit>");
    vec![RecoveryHint {
        kind: "recreate_worktree".to_string(),
        description:
            "If the branch still exists, a human may recreate a linked worktree from the branch."
                .to_string(),
        reference_command: vec![
            "git".to_string(),
            "worktree".to_string(),
            "add".to_string(),
            target_path.display().to_string(),
            ref_name.to_string(),
        ],
    }]
}

pub fn compute_worktree_remove_plan_id(plan: &WorktreeRemovePlan) -> Result<String> {
    let hash_input = WorktreeRemovePlanHashInput {
        schema_version: &plan.schema_version,
        action: &plan.action,
        repository: &plan.repository,
        target: &plan.target,
        target_state: &plan.target_state,
        preconditions: &plan.preconditions,
        execution: WorktreeRemoveExecutionHashInput {
            status: &plan.execution.status,
            execute_supported: plan.execution.execute_supported,
            future_execute_eligibility: &plan.execution.future_execute_eligibility,
            raw_git_allowed: plan.execution.raw_git_allowed,
            blocked_reasons: &plan.execution.blocked_reasons,
        },
        risk: &plan.risk,
        confirmation: WorktreeRemoveConfirmationHashInput {
            required_before_execute: plan.confirmation.required_before_execute,
            reason_codes: &plan.confirmation.reason_codes,
        },
        undo_strategy: WorktreeRemoveUndoStrategyHashInput {
            kind: &plan.undo_strategy.kind,
        },
        recovery_hint_kinds: plan
            .recovery_hints
            .iter()
            .map(|hint| hint.kind.as_str())
            .collect(),
    };
    sha256_with_domain(b"super-git-worktree-remove-plan-v0.3\n", &hash_input)
}

fn sha256_with_domain<T: Serialize>(domain: &[u8], value: &T) -> Result<String> {
    let canonical = canonical_json_bytes(value)?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(canonical);
    Ok(format_digest(hasher.finalize().as_slice()))
}

fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let value = serde_json::to_value(value)?;
    serde_json::to_vec(&sort_json_value(value)).map_err(Into::into)
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
struct WorktreeRemovePlanHashInput<'a> {
    schema_version: &'a str,
    action: &'a WorktreeRemoveAction,
    repository: &'a WorktreeRemoveRepository,
    target: &'a WorktreeRemoveTarget,
    target_state: &'a WorktreeRemoveTargetState,
    preconditions: &'a [WorktreeRemovePrecondition],
    execution: WorktreeRemoveExecutionHashInput<'a>,
    risk: &'a ActionRisk,
    confirmation: WorktreeRemoveConfirmationHashInput<'a>,
    undo_strategy: WorktreeRemoveUndoStrategyHashInput<'a>,
    recovery_hint_kinds: Vec<&'a str>,
}

#[derive(Serialize)]
struct WorktreeRemoveExecutionHashInput<'a> {
    status: &'a str,
    execute_supported: bool,
    future_execute_eligibility: &'a str,
    raw_git_allowed: bool,
    blocked_reasons: &'a [WorktreeBlockedReason],
}

#[derive(Serialize)]
struct WorktreeRemoveConfirmationHashInput<'a> {
    required_before_execute: bool,
    reason_codes: &'a [String],
}

#[derive(Serialize)]
struct WorktreeRemoveUndoStrategyHashInput<'a> {
    kind: &'a str,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::model::{
        DestructivePreviewExecution, Operation, PreviewConfirmation, RecoveryHint,
        UnavailableUndoStrategy, WorktreeRemoveAction, WorktreeRemoveOptions, WorktreeRemovePlan,
        WorktreeRemovePrecondition, WorktreeRemoveRepository, WorktreeRemoveTarget,
        WorktreeRemoveTargetState, WorktreeRemoveWorkingTree,
        DESTRUCTIVE_PREVIEW_PLAN_SCHEMA_VERSION,
    };

    use super::*;

    #[test]
    fn worktree_remove_plan_id_ignores_advisory_fields() {
        let mut plan = sample_plan();
        let first = compute_worktree_remove_plan_id(&plan).expect("plan id");

        plan.effects = vec!["Different prose.".to_string()];
        plan.limitations = vec!["Different limitation.".to_string()];
        plan.reference_commands.commands = vec![vec!["git".to_string(), "status".to_string()]];
        plan.confirmation.human_prompt = "Different prompt.".to_string();
        plan.undo_strategy.reason = "Different undo prose.".to_string();
        plan.recovery_hints[0].description = "Different recovery prose.".to_string();
        plan.recovery_hints[0].reference_command = vec!["git".to_string(), "status".to_string()];

        let second = compute_worktree_remove_plan_id(&plan).expect("plan id");

        assert_eq!(first, second);
    }

    fn sample_plan() -> WorktreeRemovePlan {
        WorktreeRemovePlan {
            schema_version: DESTRUCTIVE_PREVIEW_PLAN_SCHEMA_VERSION.to_string(),
            plan_id: String::new(),
            action: WorktreeRemoveAction {
                kind: ACTION_WORKTREE_REMOVE.to_string(),
                options: WorktreeRemoveOptions {
                    worktree: PathBuf::from("/repo.worktrees/repo__feature"),
                },
            },
            repository: WorktreeRemoveRepository {
                family_id: "sha256:family".to_string(),
                git_common_dir: PathBuf::from("/repo/.git"),
                main_worktree: Some(PathBuf::from("/repo")),
                selected_from: PathBuf::from("/repo"),
            },
            target: WorktreeRemoveTarget {
                input_path: PathBuf::from("/repo.worktrees/repo__feature"),
                canonical_path: PathBuf::from("/repo.worktrees/repo__feature"),
                worktree_list_path: PathBuf::from("/repo.worktrees/repo__feature"),
                kind: "linked".to_string(),
                worktree_git_dir: Some(PathBuf::from("/repo/.git/worktrees/repo__feature")),
                git_common_dir: Some(PathBuf::from("/repo/.git")),
                head: Some("abc123".to_string()),
                branch: Some("feature".to_string()),
                detached: false,
                locked: false,
                prunable: false,
                is_current_worktree: false,
                has_submodules: false,
            },
            target_state: WorktreeRemoveTargetState {
                operation: Operation::None,
                working_tree: WorktreeRemoveWorkingTree {
                    clean: true,
                    staged: 0,
                    unstaged: 0,
                    untracked: 0,
                    ignored: 0,
                    conflict_count: 0,
                    conflicts: Vec::new(),
                },
            },
            preconditions: vec![WorktreeRemovePrecondition {
                code: "target_is_linked_worktree".to_string(),
                status: "passed".to_string(),
            }],
            execution: DestructivePreviewExecution {
                status: "preview_only".to_string(),
                execute_supported: true,
                future_execute_eligibility: "needs_human_confirmation".to_string(),
                raw_git_allowed: false,
                suggested_super_git_command: None,
                blocked_reasons: Vec::new(),
            },
            risk: ActionRisk {
                severity: "high".to_string(),
                reversibility: "not_automatically_reversible".to_string(),
                requires_human_confirmation: true,
            },
            confirmation: PreviewConfirmation {
                required_before_execute: true,
                reason_codes: vec!["no_automatic_undo".to_string()],
                human_prompt: "Remove linked worktree?".to_string(),
            },
            effects: vec!["Delete worktree.".to_string()],
            limitations: vec!["Cannot detect processes.".to_string()],
            reference_commands: WorktreeReferenceCommands {
                semantics: "documentation_only".to_string(),
                never_execute_directly: true,
                commands: vec![vec!["git".to_string(), "worktree".to_string()]],
            },
            undo_strategy: UnavailableUndoStrategy {
                kind: "not_available".to_string(),
                reason: "No automatic undo.".to_string(),
            },
            recovery_hints: vec![RecoveryHint {
                kind: "recreate_worktree".to_string(),
                description: "Recreate it.".to_string(),
                reference_command: vec![
                    "git".to_string(),
                    "worktree".to_string(),
                    "add".to_string(),
                ],
            }],
        }
    }
}
