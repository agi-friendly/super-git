use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::git::history_edit::{
    self, HistoryEditCommit, HistoryEditProgram, HistoryEditScan, InstructionValidation,
};
use crate::model::{
    ActionRisk, HistoryEditAction, HistoryEditBlockedReason, HistoryEditExecution,
    HistoryEditInstructionsTemplate, HistoryEditOptions, HistoryEditPlan, HistoryEditPlanBranch,
    HistoryEditPlanCommit, HistoryEditPlanInstructionItem, HistoryEditPlanInstructions,
    HistoryEditPlanRange, HistoryEditPlanRepository, HistoryEditPlanWarning,
    HistoryEditPrecondition, HistoryEditPublishedScan, HistoryEditResultSummaryView,
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
    let validation = match instructions_bytes {
        Some(bytes) => {
            let document = history_edit::parse_instructions(bytes)?;
            Some(history_edit::validate_instructions(
                &scan.range.commits,
                &document,
            ))
        }
        None => None,
    };

    Ok(build_plan(base, scan, validation))
}

fn build_plan(
    base: &str,
    scan: HistoryEditScan,
    validation: Option<InstructionValidation>,
) -> HistoryEditPlan {
    let instructions_provided = validation.is_some();
    let instruction_blocks = validation
        .as_ref()
        .map(|validation| validation.blocks.clone())
        .unwrap_or_default();
    let program = validation.and_then(|validation| validation.program);

    let mut block_codes: HashSet<String> =
        scan.blocks.iter().map(|block| block.code.clone()).collect();
    block_codes.extend(instruction_blocks.iter().map(|block| block.code.clone()));
    let has_blocks = !block_codes.is_empty();

    let published_in_range = scan.range.commits.iter().any(|commit| commit.published);

    // Status is the single switch the rest of the plan reads from.
    let status = if !instructions_provided {
        if scan.blocks.is_empty() {
            "survey"
        } else {
            "blocked"
        }
    } else if has_blocks {
        "blocked"
    } else if published_in_range {
        "preview_only"
    } else {
        "executable"
    };

    let requires_confirmation_artifact = status == "preview_only";
    let executable = status == "executable" || status == "preview_only";

    let blocked_reasons = blocked_reasons(&scan, &instruction_blocks);
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
        severity: if published_in_range { "high" } else { "medium" }.to_string(),
        reversibility: "reversible_if_unchanged".to_string(),
        requires_human_confirmation: published_in_range,
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
        Some(PreviewConfirmation {
            required_before_execute: true,
            reason_codes: vec![
                "rewrites_published_commits".to_string(),
                "remote_branches_will_diverge".to_string(),
                "local_undo_does_not_unpublish".to_string(),
            ],
            human_prompt: format!("Rewrite published history on {branch_label}?"),
            required_phrase: scan
                .branch
                .as_ref()
                .map(|branch| confirmation_phrase(&branch.ref_name, &branch.tip_commit)),
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
        preconditions: preconditions(&block_codes, instructions_provided),
        execution,
        risk,
        confirmation,
        warnings,
        effects: effects(status, program.as_ref(), &branch_label),
        limitations: limitations(),
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
        undo_strategy: HistoryEditUndoStrategy {
            kind: "restore_branch_tip_snapshot".to_string(),
            deletes_branch: false,
            deletes_history: false,
        },
        undo_preview: HistoryEditUndoPreview {
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
        },
    };
    plan.plan_id = compute_history_edit_plan_id(&plan);
    plan
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
        final_tree_unchanged: program.summary.final_tree_unchanged,
    }
}

fn blocked_reasons(
    scan: &HistoryEditScan,
    instruction_blocks: &[history_edit::HistoryEditBlock],
) -> Vec<HistoryEditBlockedReason> {
    scan.blocks
        .iter()
        .chain(instruction_blocks.iter())
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

fn effects(status: &str, program: Option<&HistoryEditProgram>, branch_label: &str) -> Vec<String> {
    match (status, program) {
        ("executable" | "preview_only", Some(program)) => {
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
        ("survey", _) => vec![
            "Survey only: no instructions were provided, so no write is planned.".to_string(),
            "range.commits is the template the instruction list must follow.".to_string(),
        ],
        _ => vec!["No write is allowed until blocked reasons are resolved.".to_string()],
    }
}

fn limitations() -> Vec<String> {
    vec![
        "Published detection only sees local remote-tracking refs from the last fetch.".to_string(),
        "Undo depends on the previous tip staying reachable in the local object store.".to_string(),
        "Rewritten commits do not preserve GPG/SSH signatures from the originals.".to_string(),
    ]
}

/// The deterministic typed phrase a published-range history_edit confirmation
/// must carry. Shared by preview (which advertises it in the plan) and execute
/// (which enforces it), so the two can never drift.
pub fn confirmation_phrase(branch_ref: &str, tip_commit: &str) -> String {
    format!("rewrite published history on {branch_ref} at {tip_commit}")
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
    sha256_with_domain(b"super-git-plan-v0.4\n", &hash_input)
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

        assert_eq!(plan.schema_version, "super-git.plan.v0.4");
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
