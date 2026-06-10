use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::store::repository_id;
use crate::git::command::Git;
use crate::git::{state, status};
use crate::model::Operation;
use crate::{Result, SuperGitError};

const ACTION_HISTORY_EDIT: &str = "history_edit";

pub const INSTRUCTIONS_SCHEMA_VERSION: &str = "super-git.instructions.v0.1";

/// The cap is a guardrail against a wrong `--base` (for example, pointing at
/// the root commit by accident), which would otherwise ask an agent to
/// enumerate an entire history as instructions.
pub const HISTORY_EDIT_RANGE_CAP: usize = 100;

/// Published detection only sees refs from the last fetch; `super-git` never
/// fetches on its own, so the basis is recorded for honest plans.
pub const PUBLISHED_SCAN_BASIS: &str = "local_remote_tracking_refs";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEditScan {
    pub repository: HistoryEditRepository,
    pub branch: Option<HistoryEditBranch>,
    pub head_commit: String,
    pub operation: Operation,
    pub working_tree: HistoryEditWorkingTree,
    pub commit_signing_enabled: bool,
    pub committer_identity_configured: bool,
    pub range: HistoryEditRange,
    pub blocks: Vec<HistoryEditBlock>,
    pub warnings: Vec<HistoryEditWarning>,
    pub execution_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEditRepository {
    pub family_id: String,
    pub git_common_dir: PathBuf,
    pub worktree_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEditBranch {
    pub ref_name: String,
    pub short_name: String,
    pub tip_commit: String,
    pub checked_out_at: PathBuf,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEditWorkingTree {
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub conflict_count: u32,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEditRange {
    pub base_input: String,
    pub base_commit: String,
    pub base_is_ancestor_of_head: bool,
    pub order: String,
    pub commit_count: usize,
    pub published_scan_basis: String,
    /// Detailed entries exist only when the range is enumerable: ancestor
    /// base, non-empty, and within the cap. Blocked scans keep `commit_count`
    /// accurate so agents see why enumeration was skipped.
    pub commits: Vec<HistoryEditCommit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEditCommit {
    pub commit: String,
    pub subject: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub author_date: String,
    pub published: bool,
    pub signed: bool,
    pub is_merge: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEditBlock {
    pub code: String,
    pub severity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEditWarning {
    pub code: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstructionsDocument {
    pub schema_version: String,
    pub action: String,
    #[serde(default)]
    pub base: Option<String>,
    pub items: Vec<InstructionItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstructionItem {
    pub commit: String,
    pub op: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstructionValidation {
    pub blocks: Vec<HistoryEditBlock>,
    pub program: Option<HistoryEditProgram>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEditProgram {
    pub steps: Vec<ResolvedInstruction>,
    pub groups: Vec<HistoryEditGroup>,
    pub summary: HistoryEditResultSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedInstruction {
    pub commit: String,
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// One result commit: a kept instruction plus the fold chain attached to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEditGroup {
    pub primary_commit: String,
    pub folded_commits: Vec<String>,
    pub final_message: String,
    pub message_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEditResultSummary {
    pub commits_before: u32,
    pub commits_after: u32,
    pub messages_changed: u32,
    pub commits_folded: u32,
    /// Leading original commits that keep their object ids because nothing
    /// before or at their position changes. C8-C reuses them as-is.
    pub unchanged_prefix_commits: u32,
    pub final_tree_unchanged: bool,
}

pub fn scan_history_edit_range(current_path: &Path, base_input: &str) -> Result<HistoryEditScan> {
    validate_base_input(base_input)?;

    let git = Git::default();
    let worktree_root = worktree_root(&git, current_path)?;
    let git_common_dir = git_common_dir(&git, &worktree_root)?;
    let head_commit = rev_parse_commit(&git, &worktree_root, "HEAD^{commit}").ok_or_else(|| {
        SuperGitError::PreviewPreconditionFailed {
            action: ACTION_HISTORY_EDIT.to_string(),
            code: "head_unborn".to_string(),
            message: "history edit requires at least one commit on HEAD".to_string(),
        }
    })?;
    let base_commit = rev_parse_commit(&git, &worktree_root, &format!("{base_input}^{{commit}}"))
        .ok_or_else(|| SuperGitError::PreviewPreconditionFailed {
        action: ACTION_HISTORY_EDIT.to_string(),
        code: "base_not_resolvable".to_string(),
        message: format!("--base {base_input} does not resolve to a commit"),
    })?;

    let branch = read_branch(&git, &worktree_root, &head_commit)?;
    let operation = state::detect_operation(&git, &worktree_root)?;
    let working_tree = read_working_tree(&git, &worktree_root)?;
    let commit_signing_enabled = config_bool_true(&git, &worktree_root, "commit.gpgsign");
    let committer_identity_configured = config_value_set(&git, &worktree_root, "user.name")
        && config_value_set(&git, &worktree_root, "user.email");

    let base_is_ancestor = is_ancestor(&git, &worktree_root, &base_commit)?;
    let commit_count = if base_is_ancestor {
        count_range_commits(&git, &worktree_root, &base_commit)?
    } else {
        0
    };
    let commits = if base_is_ancestor && commit_count > 0 && commit_count <= HISTORY_EDIT_RANGE_CAP
    {
        read_range_commits(&git, &worktree_root, &base_commit)?
    } else {
        Vec::new()
    };

    let range = HistoryEditRange {
        base_input: base_input.to_string(),
        base_commit,
        base_is_ancestor_of_head: base_is_ancestor,
        order: "oldest_first".to_string(),
        commit_count,
        published_scan_basis: PUBLISHED_SCAN_BASIS.to_string(),
        commits,
    };

    let blocks = collect_scan_blocks(
        &branch,
        operation,
        &working_tree,
        commit_signing_enabled,
        committer_identity_configured,
        &range,
    );
    let warnings = collect_scan_warnings(&working_tree, &range);
    let execution_status = if blocks.is_empty() {
        "survey"
    } else {
        "blocked"
    }
    .to_string();

    Ok(HistoryEditScan {
        repository: HistoryEditRepository {
            family_id: repository_id(&git_common_dir),
            git_common_dir,
            worktree_root,
        },
        branch,
        head_commit,
        operation,
        working_tree,
        commit_signing_enabled,
        committer_identity_configured,
        range,
        blocks,
        warnings,
        execution_status,
    })
}

pub fn parse_instructions(bytes: &[u8]) -> Result<InstructionsDocument> {
    let document: InstructionsDocument = serde_json::from_slice(bytes)?;
    if document.schema_version != INSTRUCTIONS_SCHEMA_VERSION {
        return Err(SuperGitError::PreviewPreconditionFailed {
            action: ACTION_HISTORY_EDIT.to_string(),
            code: "instructions_schema_unsupported".to_string(),
            message: format!(
                "unsupported instructions schema version: {} (expected {})",
                document.schema_version, INSTRUCTIONS_SCHEMA_VERSION
            ),
        });
    }
    if document.action != ACTION_HISTORY_EDIT {
        return Err(SuperGitError::PreviewPreconditionFailed {
            action: ACTION_HISTORY_EDIT.to_string(),
            code: "instructions_action_unsupported".to_string(),
            message: format!(
                "unsupported instructions action: {} (expected {})",
                document.action, ACTION_HISTORY_EDIT
            ),
        });
    }
    Ok(document)
}

pub fn validate_instructions(
    range_commits: &[HistoryEditCommit],
    document: &InstructionsDocument,
) -> InstructionValidation {
    let mut blocks = Vec::new();

    let supported_ops = ["pick", "reword", "squash", "fixup"];
    let unsupported: Vec<&str> = document
        .items
        .iter()
        .map(|item| item.op.as_str())
        .filter(|op| !supported_ops.contains(op))
        .collect();
    if !unsupported.is_empty() {
        push_block(
            &mut blocks,
            "instruction_op_unsupported",
            Some(json!({ "ops": dedup_preserving_order(&unsupported) })),
        );
    }

    let resolutions: Vec<Option<&HistoryEditCommit>> = document
        .items
        .iter()
        .map(|item| resolve_range_commit(range_commits, &item.commit))
        .collect();
    let unknown: Vec<&str> = document
        .items
        .iter()
        .zip(&resolutions)
        .filter(|(_, resolved)| resolved.is_none())
        .map(|(item, _)| item.commit.as_str())
        .collect();
    if !unknown.is_empty() {
        push_block(
            &mut blocks,
            "instructions_unknown_commit",
            Some(json!({ "inputs": unknown })),
        );
    }

    let resolved_oids: Vec<&str> = resolutions
        .iter()
        .flatten()
        .map(|commit| commit.commit.as_str())
        .collect();
    let mut seen = HashSet::new();
    let duplicates: Vec<&str> = resolved_oids
        .iter()
        .filter(|oid| !seen.insert(**oid))
        .copied()
        .collect();
    if !duplicates.is_empty() {
        push_block(
            &mut blocks,
            "instructions_duplicate_commit",
            Some(json!({ "commits": dedup_preserving_order(&duplicates) })),
        );
    }

    let resolved_set: HashSet<&str> = resolved_oids.iter().copied().collect();
    let missing: Vec<&str> = range_commits
        .iter()
        .map(|commit| commit.commit.as_str())
        .filter(|oid| !resolved_set.contains(oid))
        .collect();
    if !missing.is_empty() {
        push_block(
            &mut blocks,
            "instructions_incomplete",
            Some(json!({ "missing_commits": missing })),
        );
    }

    let covers_range_exactly = unknown.is_empty()
        && duplicates.is_empty()
        && missing.is_empty()
        && resolved_oids.len() == range_commits.len();
    if covers_range_exactly {
        let range_order: Vec<&str> = range_commits
            .iter()
            .map(|commit| commit.commit.as_str())
            .collect();
        if resolved_oids != range_order {
            push_block(
                &mut blocks,
                "instructions_order_mismatch",
                Some(json!({ "expected_order": range_order })),
            );
        }
    }

    if document
        .items
        .first()
        .is_some_and(|item| is_fold_op(&item.op))
    {
        push_block(&mut blocks, "instruction_fold_without_predecessor", None);
    }

    for item in &document.items {
        let requires_message = item.op == "reword" || item.op == "squash";
        match (&item.message, requires_message) {
            (None, true) => {
                push_block(
                    &mut blocks,
                    "instruction_message_missing",
                    Some(json!({ "commit": item.commit, "op": item.op })),
                );
            }
            (Some(message), true) if message.trim().is_empty() => {
                push_block(
                    &mut blocks,
                    "instruction_message_empty",
                    Some(json!({ "commit": item.commit, "op": item.op })),
                );
            }
            // A message on pick/fixup would be silently ignored; refusing it
            // keeps agent intent and actual effect aligned.
            (Some(_), false) => {
                push_block(
                    &mut blocks,
                    "instruction_message_unexpected",
                    Some(json!({ "commit": item.commit, "op": item.op })),
                );
            }
            _ => {}
        }
    }

    if !document.items.is_empty() && document.items.iter().all(|item| item.op == "pick") {
        push_block(&mut blocks, "instructions_no_effective_change", None);
    }

    let program = if blocks.is_empty() && covers_range_exactly && !range_commits.is_empty() {
        Some(build_program(range_commits, document))
    } else {
        None
    };

    InstructionValidation { blocks, program }
}

fn build_program(
    range_commits: &[HistoryEditCommit],
    document: &InstructionsDocument,
) -> HistoryEditProgram {
    let mut steps = Vec::new();
    let mut groups: Vec<HistoryEditGroup> = Vec::new();

    for item in &document.items {
        let commit = resolve_range_commit(range_commits, &item.commit)
            .expect("validated instruction must resolve");
        let normalized_message = item.message.as_deref().map(normalize_message);
        steps.push(ResolvedInstruction {
            commit: commit.commit.clone(),
            op: item.op.clone(),
            message: normalized_message.clone(),
        });

        match item.op.as_str() {
            "pick" | "reword" => {
                let message_changed = item.op == "reword";
                groups.push(HistoryEditGroup {
                    primary_commit: commit.commit.clone(),
                    folded_commits: Vec::new(),
                    final_message: normalized_message.unwrap_or_else(|| commit.message.clone()),
                    message_changed,
                });
            }
            "squash" | "fixup" => {
                let group = groups.last_mut().expect("fold has validated predecessor");
                group.folded_commits.push(commit.commit.clone());
                if item.op == "squash" {
                    // When a fold group carries several message-bearing ops,
                    // the last one wins; that keeps the result deterministic
                    // without an editor to merge messages.
                    group.final_message =
                        normalized_message.expect("validated squash carries a message");
                    group.message_changed = true;
                }
            }
            other => unreachable!("validated op must be supported: {other}"),
        }
    }

    let commits_folded: u32 = groups
        .iter()
        .map(|group| group.folded_commits.len() as u32)
        .sum();
    let messages_changed = groups.iter().filter(|group| group.message_changed).count() as u32;
    let unchanged_prefix_commits = groups
        .iter()
        .take_while(|group| !group.message_changed && group.folded_commits.is_empty())
        .count() as u32;

    let summary = HistoryEditResultSummary {
        commits_before: range_commits.len() as u32,
        commits_after: groups.len() as u32,
        messages_changed,
        commits_folded,
        unchanged_prefix_commits,
        // The supported op set reuses original tree objects only, so the
        // branch tip tree is identical by construction.
        final_tree_unchanged: true,
    };

    HistoryEditProgram {
        steps,
        groups,
        summary,
    }
}

fn resolve_range_commit<'a>(
    range_commits: &'a [HistoryEditCommit],
    input: &str,
) -> Option<&'a HistoryEditCommit> {
    let needle = input.to_ascii_lowercase();
    // Accept up to 64 hex chars so full SHA-256 object ids resolve, matching the
    // undo side; SHA-1 (40) and abbreviations stay valid.
    if needle.len() < 4 || needle.len() > 64 || !needle.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut matches = range_commits
        .iter()
        .filter(|commit| commit.commit.starts_with(&needle));
    // Resolution is scoped to the range on purpose: an abbreviation that is
    // ambiguous repository-wide can still be unique among editable commits.
    match (matches.next(), matches.next()) {
        (Some(commit), None) => Some(commit),
        _ => None,
    }
}

fn is_fold_op(op: &str) -> bool {
    op == "squash" || op == "fixup"
}

fn normalize_message(raw: &str) -> String {
    format!("{}\n", raw.trim_end_matches('\n'))
}

fn dedup_preserving_order<'a>(values: &[&'a str]) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    values
        .iter()
        .filter(|value| seen.insert(**value))
        .copied()
        .collect()
}

fn collect_scan_blocks(
    branch: &Option<HistoryEditBranch>,
    operation: Operation,
    working_tree: &HistoryEditWorkingTree,
    commit_signing_enabled: bool,
    committer_identity_configured: bool,
    range: &HistoryEditRange,
) -> Vec<HistoryEditBlock> {
    let mut blocks = Vec::new();
    if branch.is_none() {
        push_block(&mut blocks, "head_detached", None);
    }
    if operation != Operation::None {
        push_block(&mut blocks, "operation_in_progress", None);
    }
    if working_tree.conflict_count > 0 {
        push_block(
            &mut blocks,
            "conflicts_present",
            Some(json!({ "conflicts": working_tree.conflicts })),
        );
    }
    if !range.base_is_ancestor_of_head {
        push_block(
            &mut blocks,
            "base_not_ancestor_of_head",
            Some(json!({ "base_commit": range.base_commit })),
        );
    } else if range.commit_count == 0 {
        push_block(&mut blocks, "range_empty", None);
    } else if range.commit_count > HISTORY_EDIT_RANGE_CAP {
        push_block(
            &mut blocks,
            "range_too_large",
            Some(json!({
                "commit_count": range.commit_count,
                "cap": HISTORY_EDIT_RANGE_CAP,
            })),
        );
    }
    let merge_commits: Vec<&str> = range
        .commits
        .iter()
        .filter(|commit| commit.is_merge)
        .map(|commit| commit.commit.as_str())
        .collect();
    if !merge_commits.is_empty() {
        push_block(
            &mut blocks,
            "merge_commit_in_range",
            Some(json!({ "commits": merge_commits })),
        );
    }
    // Execute preserves author identity via GIT_AUTHOR_NAME/EMAIL; git rejects an
    // empty author name ("empty ident name not allowed") at commit-tree time. A
    // commit with an empty author would otherwise reach an "executable" plan that
    // can never succeed, so block it honestly up front.
    let unreadable_author_commits: Vec<&str> = range
        .commits
        .iter()
        .filter(|commit| {
            commit.author_name.trim().is_empty() || commit.author_email.trim().is_empty()
        })
        .map(|commit| commit.commit.as_str())
        .collect();
    if !unreadable_author_commits.is_empty() {
        push_block(
            &mut blocks,
            "author_identity_unreadable",
            Some(json!({ "commits": unreadable_author_commits })),
        );
    }
    if commit_signing_enabled {
        push_block(&mut blocks, "commit_signing_enabled", None);
    }
    if !committer_identity_configured {
        push_block(&mut blocks, "committer_identity_missing", None);
    }
    blocks
}

fn collect_scan_warnings(
    working_tree: &HistoryEditWorkingTree,
    range: &HistoryEditRange,
) -> Vec<HistoryEditWarning> {
    let mut warnings = Vec::new();
    if working_tree.staged > 0 || working_tree.unstaged > 0 {
        warnings.push(HistoryEditWarning {
            code: "working_tree_dirty".to_string(),
        });
    }
    if range.commits.iter().any(|commit| commit.signed) {
        warnings.push(HistoryEditWarning {
            code: "signed_commits_lose_signatures".to_string(),
        });
    }
    warnings
}

fn push_block(blocks: &mut Vec<HistoryEditBlock>, code: &str, details: Option<serde_json::Value>) {
    blocks.push(HistoryEditBlock {
        code: code.to_string(),
        severity: "hard_block".to_string(),
        details,
    });
}

fn validate_base_input(base_input: &str) -> Result<()> {
    // The base input is forwarded to Git as an argument, so option-like or
    // empty inputs are rejected before any process is spawned.
    if base_input.trim().is_empty() || base_input.starts_with('-') {
        return Err(SuperGitError::PreviewPreconditionFailed {
            action: ACTION_HISTORY_EDIT.to_string(),
            code: "base_input_invalid".to_string(),
            message: format!("--base must be a non-empty ref or commit, got: {base_input:?}"),
        });
    }
    Ok(())
}

fn worktree_root(git: &Git, path: &Path) -> Result<PathBuf> {
    git.run_path_in(path, ["rev-parse", "--show-toplevel"])
}

fn git_common_dir(git: &Git, path: &Path) -> Result<PathBuf> {
    git.run_path_in(
        path,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
}

fn rev_parse_commit(git: &Git, path: &Path, spec: &str) -> Option<String> {
    git.try_run_in(path, ["rev-parse", "--verify", "--quiet", spec])
        .ok()
        .filter(|output| output.success)
        .map(|output| output.stdout.trim().to_string())
        .filter(|oid| !oid.is_empty())
}

fn read_branch(git: &Git, root: &Path, head_commit: &str) -> Result<Option<HistoryEditBranch>> {
    let symbolic = git.try_run_in(root, ["symbolic-ref", "--quiet", "HEAD"])?;
    if !symbolic.success {
        return Ok(None);
    }
    let ref_name = symbolic.stdout.trim().to_string();
    let short_name = ref_name
        .strip_prefix("refs/heads/")
        .unwrap_or(&ref_name)
        .to_string();
    let upstream = git
        .try_run_in(root, ["rev-parse", "--symbolic-full-name", "@{upstream}"])
        .ok()
        .filter(|output| output.success)
        .map(|output| output.stdout.trim().to_string())
        .filter(|name| !name.is_empty());
    Ok(Some(HistoryEditBranch {
        ref_name,
        short_name,
        tip_commit: head_commit.to_string(),
        checked_out_at: root.to_path_buf(),
        upstream,
    }))
}

fn is_ancestor(git: &Git, root: &Path, base_commit: &str) -> Result<bool> {
    let result = git.try_run_in(root, ["merge-base", "--is-ancestor", base_commit, "HEAD"])?;
    Ok(result.success)
}

fn count_range_commits(git: &Git, root: &Path, base_commit: &str) -> Result<usize> {
    let output = git.run_in(
        root,
        ["rev-list", "--count", &format!("{base_commit}..HEAD")],
    )?;
    output
        .stdout
        .trim()
        .parse::<usize>()
        .map_err(|_| SuperGitError::PreviewPreconditionFailed {
            action: ACTION_HISTORY_EDIT.to_string(),
            code: "range_count_unreadable".to_string(),
            message: format!("could not parse rev-list count: {}", output.stdout.trim()),
        })
}

fn read_range_commits(git: &Git, root: &Path, base_commit: &str) -> Result<Vec<HistoryEditCommit>> {
    let unpublished = read_unpublished_commits(git, root, base_commit)?;
    // Field separator x1f and record separator NUL cannot appear in commit
    // metadata, and the raw body is the final field so splitn keeps it whole.
    let format = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%B";
    let output = git.run_in(
        root,
        [
            "-c",
            "log.showSignature=false",
            "log",
            "--reverse",
            "-z",
            format,
            &format!("{base_commit}..HEAD"),
        ],
    )?;

    let mut commits = Vec::new();
    for record in output
        .stdout
        .split('\0')
        .filter(|record| !record.is_empty())
    {
        let fields: Vec<&str> = record.splitn(6, '\x1f').collect();
        if fields.len() != 6 {
            return Err(SuperGitError::PreviewPreconditionFailed {
                action: ACTION_HISTORY_EDIT.to_string(),
                code: "range_commit_unreadable".to_string(),
                message: "could not parse commit metadata record".to_string(),
            });
        }
        let oid = fields[0].to_string();
        let signed = commit_has_signature(git, root, &oid)?;
        commits.push(HistoryEditCommit {
            subject: fields[5].lines().next().unwrap_or("").to_string(),
            message: fields[5].to_string(),
            author_name: fields[2].to_string(),
            author_email: fields[3].to_string(),
            author_date: fields[4].to_string(),
            published: !unpublished.contains(oid.as_str()),
            signed,
            is_merge: fields[1].split_whitespace().count() > 1,
            commit: oid,
        });
    }
    Ok(commits)
}

fn read_unpublished_commits(git: &Git, root: &Path, base_commit: &str) -> Result<HashSet<String>> {
    let output = git.run_in(
        root,
        [
            "rev-list",
            &format!("{base_commit}..HEAD"),
            "--not",
            "--remotes",
        ],
    )?;
    Ok(output
        .stdout
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect())
}

fn commit_has_signature(git: &Git, root: &Path, oid: &str) -> Result<bool> {
    let output = git.run_in(root, ["cat-file", "commit", oid])?;
    for line in output.stdout.lines() {
        if line.is_empty() {
            break;
        }
        // Covers both `gpgsig` and `gpgsig-sha256` headers; continuation
        // lines start with a space and never match.
        if line.starts_with("gpgsig") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn config_bool_true(git: &Git, root: &Path, key: &str) -> bool {
    git.try_run_in(root, ["config", "--type=bool", "--get", key])
        .ok()
        .filter(|output| output.success)
        .is_some_and(|output| output.stdout.trim() == "true")
}

fn config_value_set(git: &Git, root: &Path, key: &str) -> bool {
    git.try_run_in(root, ["config", "--get", key])
        .ok()
        .filter(|output| output.success)
        .is_some_and(|output| !output.stdout.trim().is_empty())
}

fn read_working_tree(git: &Git, root: &Path) -> Result<HistoryEditWorkingTree> {
    // -z keeps conflict paths raw and parses newline paths; --untracked-files=all
    // pins the mode independent of status.showUntrackedFiles. Shared parser is in
    // git/status.rs.
    let output = git.run_bytes_in(
        root,
        ["status", "--porcelain=v1", "--untracked-files=all", "-z"],
    )?;
    let counts = status::classify_porcelain_z(&output.stdout);
    Ok(HistoryEditWorkingTree {
        staged: counts.staged,
        unstaged: counts.unstaged,
        untracked: counts.untracked,
        conflict_count: counts.conflict_count(),
        conflicts: counts.conflicts,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    use super::{
        parse_instructions, scan_history_edit_range, validate_instructions, HistoryEditScan,
        HISTORY_EDIT_RANGE_CAP,
    };

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
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// Local config pins identity and signing so the scan stays deterministic
    /// regardless of the developer's real global Git configuration.
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

    fn repo_with_feature_range(temp_dir: &Path) -> (PathBuf, Vec<String>) {
        let repo = temp_dir.join("repo");
        init_repo(&repo);
        git(&repo, &["checkout", "-q", "-b", "feature/login"]);
        commit_file(&repo, "a.txt", "a\n", "feat(login): add form");
        commit_file(&repo, "b.txt", "b\n", "fix typo");
        commit_file(&repo, "c.txt", "c\n", "wip");
        let oids = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
            .lines()
            .map(str::to_string)
            .collect();
        (repo, oids)
    }

    fn block_codes(scan: &HistoryEditScan) -> Vec<&str> {
        scan.blocks
            .iter()
            .map(|block| block.code.as_str())
            .collect()
    }

    fn warning_codes(scan: &HistoryEditScan) -> Vec<&str> {
        scan.warnings
            .iter()
            .map(|warning| warning.code.as_str())
            .collect()
    }

    fn instructions_json(items: &str) -> Vec<u8> {
        format!(
            r#"{{
              "schema_version": "super-git.instructions.v0.1",
              "action": "history_edit",
              "base": "main",
              "items": {items}
            }}"#
        )
        .into_bytes()
    }

    #[test]
    fn scan_survey_reports_range_and_performs_no_writes() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, oids) = repo_with_feature_range(temp_dir.path());
        let refs_before = git_stdout(&repo, &["for-each-ref"]);
        let status_before = git_stdout(&repo, &["status", "--porcelain=v1"]);

        let scan = scan_history_edit_range(&repo, "main").expect("scan");

        assert_eq!(scan.execution_status, "survey");
        assert!(
            scan.blocks.is_empty(),
            "unexpected blocks: {:?}",
            scan.blocks
        );
        assert!(scan.warnings.is_empty());
        let branch = scan.branch.as_ref().expect("branch");
        assert_eq!(branch.ref_name, "refs/heads/feature/login");
        assert_eq!(branch.short_name, "feature/login");
        assert_eq!(branch.tip_commit, oids[2]);
        assert_eq!(branch.upstream, None);
        assert_eq!(scan.range.order, "oldest_first");
        assert_eq!(scan.range.commit_count, 3);
        let commits: Vec<&str> = scan
            .range
            .commits
            .iter()
            .map(|commit| commit.commit.as_str())
            .collect();
        assert_eq!(commits, oids.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(scan.range.commits[0].subject, "feat(login): add form");
        assert_eq!(scan.range.commits[0].message, "feat(login): add form\n");
        assert_eq!(scan.range.commits[0].author_name, "test");
        assert_eq!(scan.range.commits[0].author_email, "test@example.com");
        assert!(!scan.range.commits[0].author_date.is_empty());
        assert!(scan.range.commits.iter().all(|commit| !commit.published));
        assert!(scan.range.commits.iter().all(|commit| !commit.signed));
        assert!(scan.range.commits.iter().all(|commit| !commit.is_merge));
        assert_eq!(git_stdout(&repo, &["for-each-ref"]), refs_before);
        assert_eq!(
            git_stdout(&repo, &["status", "--porcelain=v1"]),
            status_before,
            "scan must not change repository state"
        );
    }

    #[test]
    fn scan_blocks_detached_head() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, _) = repo_with_feature_range(temp_dir.path());
        git(&repo, &["checkout", "-q", "--detach"]);

        let scan = scan_history_edit_range(&repo, "main").expect("scan");

        assert!(scan.branch.is_none());
        assert!(block_codes(&scan).contains(&"head_detached"));
        assert_eq!(scan.execution_status, "blocked");
    }

    #[test]
    fn scan_blocks_diverged_base_and_empty_range() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, _) = repo_with_feature_range(temp_dir.path());
        git(&repo, &["checkout", "-q", "main"]);
        commit_file(&repo, "main.txt", "m\n", "main side");

        let diverged = scan_history_edit_range(&repo, "feature/login").expect("scan diverged");
        assert!(block_codes(&diverged).contains(&"base_not_ancestor_of_head"));
        assert_eq!(diverged.range.commit_count, 0);

        let empty = scan_history_edit_range(&repo, "HEAD").expect("scan empty");
        assert!(block_codes(&empty).contains(&"range_empty"));
    }

    #[test]
    fn scan_blocks_oversized_range_without_enumerating_commits() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let repo = temp_dir.path().join("repo");
        init_repo(&repo);
        git(&repo, &["branch", "base"]);
        for index in 0..=HISTORY_EDIT_RANGE_CAP {
            git(
                &repo,
                &["commit", "-q", "--allow-empty", "-m", &format!("c{index}")],
            );
        }

        let scan = scan_history_edit_range(&repo, "base").expect("scan");

        assert!(block_codes(&scan).contains(&"range_too_large"));
        assert_eq!(scan.range.commit_count, HISTORY_EDIT_RANGE_CAP + 1);
        assert!(scan.range.commits.is_empty());
    }

    #[test]
    fn scan_blocks_merge_commit_in_range() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let repo = temp_dir.path().join("repo");
        init_repo(&repo);
        git(&repo, &["branch", "base"]);
        git(&repo, &["checkout", "-q", "-b", "side"]);
        commit_file(&repo, "side.txt", "s\n", "side change");
        git(&repo, &["checkout", "-q", "main"]);
        commit_file(&repo, "main.txt", "m\n", "main change");
        git(
            &repo,
            &["merge", "-q", "--no-ff", "-m", "merge side", "side"],
        );

        let scan = scan_history_edit_range(&repo, "base").expect("scan");

        assert!(block_codes(&scan).contains(&"merge_commit_in_range"));
        assert!(scan.range.commits.iter().any(|commit| commit.is_merge));
    }

    #[test]
    fn scan_marks_published_commits_from_remote_tracking_refs() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, oids) = repo_with_feature_range(temp_dir.path());
        // A remote-tracking ref is enough for the published scan; no real
        // remote or network is involved.
        git(
            &repo,
            &["update-ref", "refs/remotes/origin/feature/login", &oids[1]],
        );

        let scan = scan_history_edit_range(&repo, "main").expect("scan");

        let published: Vec<bool> = scan
            .range
            .commits
            .iter()
            .map(|commit| commit.published)
            .collect();
        assert_eq!(published, vec![true, true, false]);
        assert_eq!(
            scan.range.published_scan_basis,
            "local_remote_tracking_refs"
        );
        assert_eq!(scan.execution_status, "survey");
    }

    #[test]
    fn scan_blocks_signing_config_and_empty_identity() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, _) = repo_with_feature_range(temp_dir.path());
        git(&repo, &["config", "commit.gpgsign", "true"]);
        git(&repo, &["config", "user.name", ""]);

        let scan = scan_history_edit_range(&repo, "main").expect("scan");
        let codes = block_codes(&scan);

        assert!(codes.contains(&"commit_signing_enabled"));
        assert!(codes.contains(&"committer_identity_missing"));
    }

    #[test]
    fn scan_blocks_in_progress_operation_and_conflicts() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let repo = temp_dir.path().join("repo");
        init_repo(&repo);
        commit_file(&repo, "conflict.txt", "base\n", "base conflict file");
        git(&repo, &["branch", "base"]);
        git(&repo, &["checkout", "-q", "-b", "side"]);
        commit_file(&repo, "conflict.txt", "side\n", "side change");
        git(&repo, &["checkout", "-q", "main"]);
        commit_file(&repo, "conflict.txt", "main\n", "main change");
        let merge = run_git(&repo, &["merge", "side"]);
        assert!(!merge.status.success(), "merge should conflict");

        let scan = scan_history_edit_range(&repo, "base").expect("scan");
        let codes = block_codes(&scan);

        assert!(codes.contains(&"operation_in_progress"));
        assert!(codes.contains(&"conflicts_present"));
        assert_eq!(scan.execution_status, "blocked");
    }

    #[test]
    fn scan_warns_on_dirty_tree_without_blocking() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, _) = repo_with_feature_range(temp_dir.path());
        std::fs::write(repo.join("a.txt"), "changed\n").expect("write unstaged");
        std::fs::write(repo.join("staged.txt"), "staged\n").expect("write staged");
        git(&repo, &["add", "staged.txt"]);

        let scan = scan_history_edit_range(&repo, "main").expect("scan");

        assert_eq!(scan.execution_status, "survey");
        assert!(warning_codes(&scan).contains(&"working_tree_dirty"));
        assert_eq!(scan.working_tree.staged, 1);
        assert_eq!(scan.working_tree.unstaged, 1);
    }

    #[test]
    fn scan_detects_signature_header_and_warns() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, oids) = repo_with_feature_range(temp_dir.path());
        let tree = git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]);
        // A syntactically present (not valid) signature header is enough:
        // the contract warns about signature loss, it never verifies.
        let raw = format!(
            "tree {tree}\nparent {parent}\nauthor test <test@example.com> 1700000000 +0000\ncommitter test <test@example.com> 1700000000 +0000\ngpgsig -----BEGIN PGP SIGNATURE-----\n fake\n -----END PGP SIGNATURE-----\n\nsigned: change message\n",
            parent = oids[2],
        );
        let mut hash_object = Command::new("git")
            .current_dir(&repo)
            .args(["hash-object", "-t", "commit", "-w", "--stdin"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn hash-object");
        hash_object
            .stdin
            .take()
            .expect("stdin")
            .write_all(raw.as_bytes())
            .expect("write raw commit");
        let output = hash_object.wait_with_output().expect("hash-object output");
        assert!(output.status.success());
        let signed_oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
        git(
            &repo,
            &["update-ref", "refs/heads/feature/login", &signed_oid],
        );

        let scan = scan_history_edit_range(&repo, "main").expect("scan");

        let signed_commit = scan
            .range
            .commits
            .iter()
            .find(|commit| commit.commit == signed_oid)
            .expect("signed commit in range");
        assert!(signed_commit.signed);
        assert!(warning_codes(&scan).contains(&"signed_commits_lose_signatures"));
    }

    #[test]
    fn scan_blocks_commit_with_empty_author_identity() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, oids) = repo_with_feature_range(temp_dir.path());
        let tree = git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]);
        // Raw commit whose author name is empty ("author  <email>"). commit-tree
        // would reject it with "empty ident name not allowed", so the plan must
        // be blocked rather than executable.
        let raw = format!(
            "tree {tree}\nparent {parent}\nauthor  <orphan@example.com> 1700000000 +0000\ncommitter test <test@example.com> 1700000000 +0000\n\nempty author\n",
            parent = oids[2],
        );
        let mut hash_object = Command::new("git")
            .current_dir(&repo)
            .args(["hash-object", "-t", "commit", "-w", "--stdin"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn hash-object");
        hash_object
            .stdin
            .take()
            .expect("stdin")
            .write_all(raw.as_bytes())
            .expect("write raw commit");
        let output = hash_object.wait_with_output().expect("hash-object output");
        assert!(output.status.success());
        let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
        git(&repo, &["update-ref", "refs/heads/feature/login", &oid]);

        let scan = scan_history_edit_range(&repo, "main").expect("scan");

        let empty = scan
            .range
            .commits
            .iter()
            .find(|commit| commit.commit == oid)
            .expect("empty-author commit in range");
        assert!(
            empty.author_name.trim().is_empty(),
            "author name parsed as empty"
        );
        assert!(block_codes(&scan).contains(&"author_identity_unreadable"));
        assert_eq!(scan.execution_status, "blocked");
    }

    #[test]
    fn scan_rejects_unresolvable_or_option_like_base() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, _) = repo_with_feature_range(temp_dir.path());

        let unresolvable =
            scan_history_edit_range(&repo, "no-such-ref").expect_err("unresolvable base");
        assert!(unresolvable.to_string().contains("base_not_resolvable"));

        let option_like = scan_history_edit_range(&repo, "--all").expect_err("option-like base");
        assert!(option_like.to_string().contains("base_input_invalid"));
    }

    #[test]
    fn parse_rejects_wrong_schema_or_action() {
        let wrong_schema = br#"{"schema_version":"super-git.instructions.v9.9","action":"history_edit","items":[]}"#;
        let error = parse_instructions(wrong_schema).expect_err("wrong schema");
        assert!(error
            .to_string()
            .contains("instructions_schema_unsupported"));

        let wrong_action = br#"{"schema_version":"super-git.instructions.v0.1","action":"worktree_remove","items":[]}"#;
        let error = parse_instructions(wrong_action).expect_err("wrong action");
        assert!(error
            .to_string()
            .contains("instructions_action_unsupported"));
    }

    #[test]
    fn validate_builds_program_with_folds_and_unchanged_prefix() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, oids) = repo_with_feature_range(temp_dir.path());
        let scan = scan_history_edit_range(&repo, "main").expect("scan");
        // Abbreviated ids must resolve against the range and freeze to full
        // object ids in the program.
        let document = parse_instructions(&instructions_json(&format!(
            r#"[
              {{ "commit": "{p0}", "op": "pick" }},
              {{ "commit": "{p1}", "op": "reword", "message": "feat(login): validate email" }},
              {{ "commit": "{p2}", "op": "fixup" }}
            ]"#,
            p0 = &oids[0][..8],
            p1 = &oids[1][..8],
            p2 = &oids[2][..8],
        )))
        .expect("parse");

        let validation = validate_instructions(&scan.range.commits, &document);

        assert!(validation.blocks.is_empty(), "{:?}", validation.blocks);
        let program = validation.program.expect("program");
        assert_eq!(program.steps[1].commit, oids[1]);
        assert_eq!(
            program.steps[1].message.as_deref(),
            Some("feat(login): validate email\n"),
            "messages are normalized to one trailing newline"
        );
        assert_eq!(program.groups.len(), 2);
        assert_eq!(program.groups[0].primary_commit, oids[0]);
        assert!(!program.groups[0].message_changed);
        assert_eq!(program.groups[1].primary_commit, oids[1]);
        assert_eq!(program.groups[1].folded_commits, vec![oids[2].clone()]);
        assert_eq!(
            program.groups[1].final_message,
            "feat(login): validate email\n"
        );
        assert_eq!(program.summary.commits_before, 3);
        assert_eq!(program.summary.commits_after, 2);
        assert_eq!(program.summary.messages_changed, 1);
        assert_eq!(program.summary.commits_folded, 1);
        assert_eq!(program.summary.unchanged_prefix_commits, 1);
        assert!(program.summary.final_tree_unchanged);
    }

    #[test]
    fn validate_last_message_bearing_op_wins_in_a_fold_group() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, oids) = repo_with_feature_range(temp_dir.path());
        let scan = scan_history_edit_range(&repo, "main").expect("scan");
        let document = parse_instructions(&instructions_json(&format!(
            r#"[
              {{ "commit": "{}", "op": "pick" }},
              {{ "commit": "{}", "op": "squash", "message": "first" }},
              {{ "commit": "{}", "op": "squash", "message": "second" }}
            ]"#,
            oids[0], oids[1], oids[2],
        )))
        .expect("parse");

        let validation = validate_instructions(&scan.range.commits, &document);

        let program = validation.program.expect("program");
        assert_eq!(program.groups.len(), 1);
        assert_eq!(program.groups[0].final_message, "second\n");
        assert_eq!(program.summary.commits_after, 1);
        assert_eq!(program.summary.commits_folded, 2);
        assert_eq!(program.summary.messages_changed, 1);
        assert_eq!(program.summary.unchanged_prefix_commits, 0);
    }

    #[test]
    fn validate_blocks_unknown_duplicate_incomplete_and_order_mismatch() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, oids) = repo_with_feature_range(temp_dir.path());
        let scan = scan_history_edit_range(&repo, "main").expect("scan");

        let unknown_and_duplicate = parse_instructions(&instructions_json(&format!(
            r#"[
              {{ "commit": "{p0}", "op": "pick" }},
              {{ "commit": "{p0}", "op": "reword", "message": "again" }},
              {{ "commit": "deadbeef", "op": "pick" }}
            ]"#,
            p0 = oids[0],
        )))
        .expect("parse");
        let validation = validate_instructions(&scan.range.commits, &unknown_and_duplicate);
        let codes: Vec<&str> = validation
            .blocks
            .iter()
            .map(|block| block.code.as_str())
            .collect();
        assert!(codes.contains(&"instructions_unknown_commit"));
        assert!(codes.contains(&"instructions_duplicate_commit"));
        assert!(codes.contains(&"instructions_incomplete"));
        assert!(validation.program.is_none());

        let swapped = parse_instructions(&instructions_json(&format!(
            r#"[
              {{ "commit": "{}", "op": "pick" }},
              {{ "commit": "{}", "op": "pick" }},
              {{ "commit": "{}", "op": "reword", "message": "m" }}
            ]"#,
            oids[1], oids[0], oids[2],
        )))
        .expect("parse");
        let validation = validate_instructions(&scan.range.commits, &swapped);
        let codes: Vec<&str> = validation
            .blocks
            .iter()
            .map(|block| block.code.as_str())
            .collect();
        assert!(codes.contains(&"instructions_order_mismatch"));
        assert!(validation.program.is_none());
    }

    #[test]
    fn validate_blocks_fold_message_and_no_change_rules() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, oids) = repo_with_feature_range(temp_dir.path());
        let scan = scan_history_edit_range(&repo, "main").expect("scan");

        let document = parse_instructions(&instructions_json(&format!(
            r#"[
              {{ "commit": "{}", "op": "fixup" }},
              {{ "commit": "{}", "op": "reword" }},
              {{ "commit": "{}", "op": "squash", "message": "   " }}
            ]"#,
            oids[0], oids[1], oids[2],
        )))
        .expect("parse");
        let validation = validate_instructions(&scan.range.commits, &document);
        let codes: Vec<&str> = validation
            .blocks
            .iter()
            .map(|block| block.code.as_str())
            .collect();
        assert!(codes.contains(&"instruction_fold_without_predecessor"));
        assert!(codes.contains(&"instruction_message_missing"));
        assert!(codes.contains(&"instruction_message_empty"));

        let all_pick = parse_instructions(&instructions_json(&format!(
            r#"[
              {{ "commit": "{}", "op": "pick" }},
              {{ "commit": "{}", "op": "pick" }},
              {{ "commit": "{}", "op": "pick" }}
            ]"#,
            oids[0], oids[1], oids[2],
        )))
        .expect("parse");
        let validation = validate_instructions(&scan.range.commits, &all_pick);
        let codes: Vec<&str> = validation
            .blocks
            .iter()
            .map(|block| block.code.as_str())
            .collect();
        assert!(codes.contains(&"instructions_no_effective_change"));
    }

    #[test]
    fn validate_blocks_unsupported_op_and_unexpected_message() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (repo, oids) = repo_with_feature_range(temp_dir.path());
        let scan = scan_history_edit_range(&repo, "main").expect("scan");

        let document = parse_instructions(&instructions_json(&format!(
            r#"[
              {{ "commit": "{}", "op": "pick", "message": "unexpected" }},
              {{ "commit": "{}", "op": "drop" }},
              {{ "commit": "{}", "op": "reword", "message": "fine" }}
            ]"#,
            oids[0], oids[1], oids[2],
        )))
        .expect("parse");
        let validation = validate_instructions(&scan.range.commits, &document);
        let codes: Vec<&str> = validation
            .blocks
            .iter()
            .map(|block| block.code.as_str())
            .collect();
        assert!(codes.contains(&"instruction_op_unsupported"));
        assert!(codes.contains(&"instruction_message_unexpected"));
        assert!(validation.program.is_none());
    }

    #[test]
    fn resolve_accepts_full_sha256_object_ids() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo");
        // SHA-256 repository: object ids are 64 hex chars, which the scan emits
        // and an agent echoes straight back into the instruction list.
        git(
            &repo,
            &["init", "-q", "-b", "main", "--object-format=sha256"],
        );
        git(&repo, &["config", "user.name", "test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        commit_file(&repo, "README.md", "hello\n", "initial");
        git(&repo, &["checkout", "-q", "-b", "feature"]);
        commit_file(&repo, "a.txt", "a\n", "feat: a");
        commit_file(&repo, "b.txt", "b\n", "fix: b");
        let oids: Vec<String> = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(oids[0].len(), 64, "sha256 object id is 64 hex chars");

        let scan = scan_history_edit_range(&repo, "main").expect("scan");
        let document = parse_instructions(&instructions_json(&format!(
            r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"reworded"}}]"#,
            oids[0], oids[1]
        )))
        .expect("parse");

        let validation = validate_instructions(&scan.range.commits, &document);

        assert!(
            validation.blocks.is_empty(),
            "full sha256 ids must resolve: {:?}",
            validation.blocks
        );
        assert!(
            validation.program.is_some(),
            "a valid sha256 instruction list yields a program"
        );
    }
}
