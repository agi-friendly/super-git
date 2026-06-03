use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::git::command::Git;
use crate::git::fingerprint::{read_state_fingerprint, resolved_stage_changes_paths};
use crate::git::state;
use crate::model::{
    ActionRisk, Operation, PreviewAction, PreviewPlan, PreviewPrecondition, UndoPreview,
    UndoStrategy, PLAN_SCHEMA_VERSION,
};
use crate::{Result, SuperGitError};

const ACTION_STAGE_CHANGES: &str = "stage_changes";

pub fn preview_stage_changes(path: &Path) -> Result<PreviewPlan> {
    let git = Git::default();
    let state = state::read_state(path)?;

    ensure_precondition(
        state.working_tree.conflict_count == 0,
        "no_conflicts",
        "working tree must not contain unmerged paths",
    )?;
    ensure_precondition(
        state.operation == Operation::None,
        "operation_none",
        "repository must not be inside an in-progress Git operation",
    )?;
    ensure_precondition(
        state.working_tree.unstaged > 0 || state.working_tree.untracked > 0,
        "has_unstaged_or_untracked_changes",
        "stage_changes requires unstaged or untracked changes",
    )?;

    let resolved_paths = resolved_stage_changes_paths(&git, &state.root)?;
    ensure_precondition(
        !resolved_paths.is_empty(),
        "has_resolved_paths",
        "stage_changes resolved no paths to stage",
    )?;

    let action = PreviewAction {
        kind: ACTION_STAGE_CHANGES.to_string(),
        scope: "all".to_string(),
        resolved_paths,
    };
    let fingerprint = read_state_fingerprint(
        &git,
        &state.root,
        &state.root,
        state.head.commit.clone(),
        state.operation,
    )?;
    let preconditions = vec![
        passed("operation_none"),
        passed("no_conflicts"),
        passed("has_unstaged_or_untracked_changes"),
    ];
    let risk = ActionRisk {
        severity: "low".to_string(),
        reversibility: "reversible".to_string(),
        requires_human_confirmation: false,
    };
    let undo_strategy = UndoStrategy {
        kind: "restore_index_snapshot".to_string(),
        requires_index_snapshot: true,
    };
    let undo_preview = UndoPreview {
        kind: "restore_index_snapshot".to_string(),
        available_after_execute: true,
    };

    let mut plan = PreviewPlan {
        schema_version: PLAN_SCHEMA_VERSION.to_string(),
        plan_id: String::new(),
        action,
        repository: state.root,
        state_fingerprint: fingerprint,
        preconditions,
        risk,
        effects: vec!["Stage unstaged and untracked changes in the current worktree.".to_string()],
        reference_commands: vec![vec![
            "git".to_string(),
            "add".to_string(),
            "--all".to_string(),
        ]],
        undo_strategy,
        undo_preview,
    };
    plan.plan_id = compute_plan_id(&plan)?;
    Ok(plan)
}

fn ensure_precondition(passed: bool, code: &str, message: &str) -> Result<()> {
    if passed {
        return Ok(());
    }

    Err(SuperGitError::PreviewPreconditionFailed {
        action: ACTION_STAGE_CHANGES.to_string(),
        code: code.to_string(),
        message: message.to_string(),
    })
}

fn passed(code: &str) -> PreviewPrecondition {
    PreviewPrecondition {
        code: code.to_string(),
        status: "passed".to_string(),
    }
}

fn compute_plan_id(plan: &PreviewPlan) -> Result<String> {
    let hash_input = PlanHashInput {
        schema_version: &plan.schema_version,
        action: &plan.action,
        repository: &plan.repository,
        state_fingerprint: &plan.state_fingerprint,
        preconditions: &plan.preconditions,
        risk: &plan.risk,
        undo_strategy: &plan.undo_strategy,
    };
    let canonical = serde_json::to_vec(&hash_input)?;

    let mut hasher = Sha256::new();
    hasher.update(b"super-git-plan-v0.1\n");
    hasher.update(canonical);
    Ok(format_digest(hasher.finalize().as_slice()))
}

/// Hash only the execution contract. Advisory fields such as effects,
/// reference_commands, and undo_preview must not affect plan identity.
#[derive(Serialize)]
struct PlanHashInput<'a> {
    schema_version: &'a str,
    action: &'a PreviewAction,
    repository: &'a std::path::Path,
    state_fingerprint: &'a crate::model::StateFingerprint,
    preconditions: &'a [PreviewPrecondition],
    risk: &'a ActionRisk,
    undo_strategy: &'a UndoStrategy,
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::model::{
        ActionRisk, Operation, PreviewAction, PreviewPlan, PreviewPrecondition, StateFingerprint,
        UndoPreview, UndoStrategy, PLAN_SCHEMA_VERSION,
    };

    use super::compute_plan_id;

    #[test]
    fn plan_id_ignores_advisory_fields() {
        let mut plan = sample_plan();
        let first = compute_plan_id(&plan).expect("plan id");

        plan.effects = vec!["Different prose only.".to_string()];
        plan.reference_commands = vec![vec!["git".to_string(), "status".to_string()]];
        plan.undo_preview.available_after_execute = false;

        let second = compute_plan_id(&plan).expect("plan id");

        assert_eq!(first, second);
    }

    fn sample_plan() -> PreviewPlan {
        PreviewPlan {
            schema_version: PLAN_SCHEMA_VERSION.to_string(),
            plan_id: String::new(),
            action: PreviewAction {
                kind: "stage_changes".to_string(),
                scope: "all".to_string(),
                resolved_paths: vec!["file.txt".to_string()],
            },
            repository: PathBuf::from("/repo"),
            state_fingerprint: StateFingerprint {
                schema_version: "super-git.fingerprint.v0.1".to_string(),
                repository: PathBuf::from("/repo"),
                head_commit: Some("abc123".to_string()),
                operation: Operation::None,
                status_porcelain_v1_z_sha256: "sha256:status".to_string(),
                staged_diff_sha256: "sha256:staged".to_string(),
                unstaged_diff_sha256: "sha256:unstaged".to_string(),
                untracked_content_sha256: "sha256:untracked".to_string(),
            },
            preconditions: vec![PreviewPrecondition {
                code: "operation_none".to_string(),
                status: "passed".to_string(),
            }],
            risk: ActionRisk {
                severity: "low".to_string(),
                reversibility: "reversible".to_string(),
                requires_human_confirmation: false,
            },
            effects: vec!["Stage changes.".to_string()],
            reference_commands: vec![vec![
                "git".to_string(),
                "add".to_string(),
                "--all".to_string(),
            ]],
            undo_strategy: UndoStrategy {
                kind: "restore_index_snapshot".to_string(),
                requires_index_snapshot: true,
            },
            undo_preview: UndoPreview {
                kind: "restore_index_snapshot".to_string(),
                available_after_execute: true,
            },
        }
    }
}
