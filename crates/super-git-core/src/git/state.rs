use std::path::{Path, PathBuf};

use crate::git::command::Git;
use crate::git::status;
use crate::git::worktree;
use crate::model::{
    HeadInfo, InspectRiskHint, InspectSummary, InspectWarning, NextAction, NextGuardrails,
    Operation, RepoState, RiskFactor, RiskLevel, UpstreamComparisonBasis, UpstreamComparisonStatus,
    UpstreamInfo, WarningSeverity, WorkingTree, WorktreeContext, WorktreeKind,
    EVALUATED_INSPECT_ACTIONS,
};
use crate::Result;

/// 저장소의 HEAD 위치와 진행 중인 작업을 한 번에 읽는다.
pub fn read_state(path: &Path) -> Result<RepoState> {
    let git = Git::default();
    let root = repo_root(&git, path)?;
    let worktree_context = read_worktree_context(&git, path)?;
    let head = read_head(&git, path)?;
    let upstream = read_upstream(&git, path)?;
    let working_tree = read_working_tree(&git, path)?;
    let operation = detect_operation(&git, path)?;
    let next = compute_next_guardrails(operation, &working_tree, &upstream);
    let warnings = compute_warnings(&upstream);
    let summary = compute_summary(operation, &working_tree, &upstream, &worktree_context);
    let risk_hint = compute_risk_hint(operation, &working_tree);
    Ok(RepoState {
        root,
        worktree_context,
        head,
        upstream,
        working_tree,
        operation,
        next,
        warnings,
        summary,
        risk_hint,
    })
}

fn read_head(git: &Git, path: &Path) -> Result<HeadInfo> {
    // symbolic-ref가 성공하면 브랜치에 붙어 있고(attached), 실패하면 detached다.
    let branch_result = git.try_run_in(path, ["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let branch = non_empty(
        branch_result
            .success
            .then(|| branch_result.stdout.trim().to_string()),
    );

    // 아직 커밋이 없는 저장소(unborn branch)에서는 rev-parse가 실패한다.
    let commit_result = git.try_run_in(path, ["rev-parse", "--verify", "--quiet", "HEAD"])?;
    let commit = non_empty(
        commit_result
            .success
            .then(|| commit_result.stdout.trim().to_string()),
    );

    // 브랜치가 없는데 커밋은 있으면 HEAD가 커밋을 직접 가리키는 detached 상태다.
    let detached = branch.is_none() && commit.is_some();

    Ok(HeadInfo {
        branch,
        commit,
        detached,
    })
}

/// upstream 추적 브랜치 이름과 ahead/behind를 읽는다. 미설정이면 None.
fn read_upstream(git: &Git, path: &Path) -> Result<Option<UpstreamInfo>> {
    let name_result = git.try_run_in(
        path,
        [
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )?;
    if !name_result.success {
        // upstream 미설정, detached HEAD, unborn branch 등.
        return Ok(None);
    }
    let name = name_result.stdout.trim().to_string();
    if name.is_empty() {
        return Ok(None);
    }

    // 출력은 "<behind>\t<ahead>". left=@{upstream}쪽(behind), right=HEAD쪽(ahead).
    let counts = git.try_run_in(
        path,
        ["rev-list", "--count", "--left-right", "@{upstream}...HEAD"],
    )?;
    let (behind, ahead, comparison_status) = if counts.success {
        match parse_ahead_behind(&counts.stdout) {
            Some((behind, ahead)) => (behind, ahead, UpstreamComparisonStatus::Ok),
            None => (0, 0, UpstreamComparisonStatus::Failed),
        }
    } else {
        (0, 0, UpstreamComparisonStatus::Failed)
    };

    Ok(Some(UpstreamInfo {
        name,
        ahead,
        behind,
        comparison_basis: UpstreamComparisonBasis::LocalTrackingRef,
        comparison_status,
    }))
}

/// `rev-list --count --left-right @{upstream}...HEAD` 출력("<behind>\t<ahead>")을 파싱한다.
fn parse_ahead_behind(output: &str) -> Option<(u32, u32)> {
    let mut parts = output.split_whitespace();
    let behind = parts.next()?.parse().ok()?;
    let ahead = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((behind, ahead))
}

fn compute_warnings(upstream: &Option<UpstreamInfo>) -> Vec<InspectWarning> {
    let mut warnings = Vec::new();
    if let Some(upstream) = upstream {
        warnings.push(InspectWarning {
            code: "upstream_freshness_unknown".to_string(),
            severity: WarningSeverity::Low,
            message: "Ahead/behind is based on the local tracking ref; fetch before treating remote state as current.".to_string(),
        });
        if upstream.comparison_status == UpstreamComparisonStatus::Failed {
            warnings.push(InspectWarning {
                code: "upstream_comparison_failed".to_string(),
                severity: WarningSeverity::Medium,
                message: "Upstream name was found, but ahead/behind comparison failed; counts should not be trusted.".to_string(),
            });
        }
    }
    warnings
}

fn compute_summary(
    operation: Operation,
    working_tree: &WorkingTree,
    upstream: &Option<UpstreamInfo>,
    worktree_context: &WorktreeContext,
) -> InspectSummary {
    let state = if working_tree.conflict_count > 0 {
        "blocked"
    } else if operation != Operation::None {
        "in_progress"
    } else if !working_tree.clean {
        "dirty"
    } else {
        "ready"
    };

    let mut codes = Vec::new();
    codes.push(operation_code(operation).to_string());
    codes.push(if working_tree.clean {
        "working_tree_clean".to_string()
    } else {
        "working_tree_dirty".to_string()
    });
    if working_tree.conflict_count > 0 {
        codes.push("conflicts_present".to_string());
    }
    codes.push(upstream_code(upstream).to_string());
    codes.push(worktree_code(worktree_context.kind).to_string());

    let message = match state {
        "blocked" => "Resolve conflicts before continuing Git operations.",
        "in_progress" => "A Git operation is in progress.",
        "dirty" => "Working tree has local changes.",
        _ => "Repository is ready for preview selection.",
    }
    .to_string();

    InspectSummary {
        state: state.to_string(),
        state_scope: "repository_posture".to_string(),
        execution_permission: "not_granted_by_inspect".to_string(),
        codes,
        message,
    }
}

fn compute_risk_hint(operation: Operation, working_tree: &WorkingTree) -> InspectRiskHint {
    let mut factors = Vec::new();
    if working_tree.conflict_count > 0 {
        factors.push(RiskFactor {
            code: "conflicts_present".to_string(),
            level: RiskLevel::High,
            message: "Unmerged paths must be resolved before continuing.".to_string(),
        });
    } else if !working_tree.clean {
        factors.push(RiskFactor {
            code: "working_tree_dirty".to_string(),
            level: RiskLevel::Medium,
            message:
                "Local changes are present; commands that touch the working tree need extra care."
                    .to_string(),
        });
    }
    if operation != Operation::None {
        factors.push(RiskFactor {
            code: "operation_in_progress".to_string(),
            level: RiskLevel::Medium,
            message: "Repository is inside an in-progress Git operation.".to_string(),
        });
    }

    let level = if factors.iter().any(|factor| factor.level == RiskLevel::High) {
        RiskLevel::High
    } else if factors
        .iter()
        .any(|factor| factor.level == RiskLevel::Medium)
    {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    };

    InspectRiskHint {
        scope: "current_state_only".to_string(),
        level,
        factors,
    }
}

fn operation_code(operation: Operation) -> &'static str {
    match operation {
        Operation::None => "operation_none",
        Operation::Merging => "operation_merging",
        Operation::Rebasing => "operation_rebasing",
        Operation::Applying => "operation_applying",
        Operation::CherryPicking => "operation_cherry_picking",
        Operation::Reverting => "operation_reverting",
        Operation::Bisecting => "operation_bisecting",
    }
}

fn upstream_code(upstream: &Option<UpstreamInfo>) -> &'static str {
    match upstream {
        None => "upstream_none",
        Some(upstream) if upstream.comparison_status == UpstreamComparisonStatus::Failed => {
            "upstream_comparison_failed"
        }
        Some(upstream) if upstream.ahead == 0 && upstream.behind == 0 => "upstream_synced",
        Some(upstream) if upstream.ahead > 0 && upstream.behind == 0 => "upstream_ahead",
        Some(upstream) if upstream.ahead == 0 && upstream.behind > 0 => "upstream_behind",
        Some(_) => "upstream_diverged",
    }
}

fn worktree_code(kind: WorktreeKind) -> &'static str {
    match kind {
        WorktreeKind::Main => "main_worktree",
        WorktreeKind::Linked => "linked_worktree",
        WorktreeKind::Bare => "bare_worktree",
        WorktreeKind::Unknown => "unknown_worktree",
    }
}

/// 워킹 트리 변경을 요약한다. 상세 목록은 status 명령에 맡기고 카운트+충돌 목록만 만든다.
fn read_working_tree(git: &Git, path: &Path) -> Result<WorkingTree> {
    let counts = status::read_porcelain_counts(git, path, false)?;
    Ok(WorkingTree {
        clean: counts.staged == 0
            && counts.unstaged == 0
            && counts.untracked == 0
            && counts.conflict_count() == 0,
        staged: counts.staged,
        unstaged: counts.unstaged,
        untracked: counts.untracked,
        conflict_count: counts.conflict_count(),
        conflicts: counts.conflicts,
    })
}

/// 현재 상태로부터 다음 행동 guardrail을 만든다.
/// git을 호출하지 않는 순수 함수라 단위테스트로 촘촘히 검증할 수 있다.
fn compute_next_guardrails(
    operation: Operation,
    working_tree: &WorkingTree,
    upstream: &Option<UpstreamInfo>,
) -> NextGuardrails {
    NextGuardrails {
        scope: "inspect_state_only".to_string(),
        execution_contract: "preview_required".to_string(),
        allowed_semantics: "preview_candidate".to_string(),
        blocked_semantics: "state_guardrail".to_string(),
        needs_human_review_scope: "evaluated_inspect_actions_only".to_string(),
        raw_git_allowed: false,
        evaluated_actions: EVALUATED_INSPECT_ACTIONS
            .iter()
            .map(|action| (*action).to_string())
            .collect(),
        allowed: compute_allowed_actions(operation, working_tree, upstream),
        blocked: compute_blocked_actions(operation, working_tree, upstream),
        needs_human_review: Vec::new(),
    }
}

fn compute_allowed_actions(
    operation: Operation,
    working_tree: &WorkingTree,
    upstream: &Option<UpstreamInfo>,
) -> Vec<NextAction> {
    let mut actions = Vec::new();

    // 진행 중인 작업이 있으면 그 작업의 해결/탈출 행동이 우선이다.
    match operation {
        Operation::None => {}
        Operation::Merging => {
            if working_tree.conflict_count > 0 {
                push_resolve_if_conflicts(&mut actions, working_tree);
            } else {
                // 충돌이 없으면 해결 완료 또는 --no-commit 상태다. 커밋해서 merge를 마친다.
                actions.push(action(
                    "merge_continue",
                    "conflicts resolved; complete the merge",
                    Some(&["git", "merge", "--continue"]),
                    None,
                ));
            }
            actions.push(action(
                "merge_abort",
                "a merge is in progress",
                Some(&["git", "merge", "--abort"]),
                Some("reversible"),
            ));
        }
        Operation::Rebasing => {
            push_resolve_if_conflicts(&mut actions, working_tree);
            push_sequence_actions(&mut actions, "rebase", "rebase", working_tree);
        }
        Operation::Applying => {
            push_resolve_if_conflicts(&mut actions, working_tree);
            push_sequence_actions(&mut actions, "am", "am", working_tree);
        }
        Operation::CherryPicking => {
            push_resolve_if_conflicts(&mut actions, working_tree);
            push_sequence_actions(&mut actions, "cherry_pick", "cherry-pick", working_tree);
        }
        Operation::Reverting => {
            push_resolve_if_conflicts(&mut actions, working_tree);
            push_sequence_actions(&mut actions, "revert", "revert", working_tree);
        }
        Operation::Bisecting => {
            actions.push(action(
                "bisect_reset",
                "a bisect session is in progress",
                Some(&["git", "bisect", "reset"]),
                Some("reversible"),
            ));
        }
    }

    // 진행 중인 작업이 없을 때만 일반 흐름(commit/push/pull)을 제안한다.
    if operation == Operation::None {
        // staged 변경이 있어야 실제 commit이 가능하다.
        if working_tree.staged > 0 {
            actions.push(action(
                "commit",
                "staged changes are ready to commit",
                None,
                None,
            ));
        }
        // unstaged/untracked는 아직 commit 대상이 아니므로 먼저 stage하도록 유도한다.
        if working_tree.unstaged > 0 || working_tree.untracked > 0 {
            actions.push(action(
                "stage_changes",
                "there are unstaged or untracked changes",
                None,
                None,
            ));
        }

        if let Some(u) = upstream {
            // pull/integrate는 워킹 트리를 건드리므로 dirty면 제안하지 않는다(commit/stash로 먼저
            // 정리하도록 유도). push는 워킹 트리에 영향이 없어 그대로 둔다.
            let clean = working_tree.clean;
            match (u.ahead, u.behind) {
                (a, 0) if a > 0 => actions.push(action(
                    "push",
                    "local is ahead of upstream",
                    Some(&["git", "push"]),
                    None,
                )),
                (0, b) if b > 0 && clean => actions.push(action(
                    "pull",
                    "upstream is ahead of local",
                    Some(&["git", "pull"]),
                    None,
                )),
                (a, b) if a > 0 && b > 0 && clean => actions.push(action(
                    "integrate_diverged",
                    "local and upstream have diverged",
                    Some(&["git", "pull", "--rebase"]),
                    None,
                )),
                _ => {}
            }
        }
    }

    actions
}

fn compute_blocked_actions(
    operation: Operation,
    working_tree: &WorkingTree,
    upstream: &Option<UpstreamInfo>,
) -> Vec<NextAction> {
    let mut blocked = Vec::new();

    if working_tree.conflict_count > 0 {
        blocked.push(action(
            "continue_operation",
            "conflicts remain; resolve all unmerged paths before continuing",
            None,
            None,
        ));
    }

    if operation == Operation::None
        && working_tree.staged == 0
        && (working_tree.unstaged > 0 || working_tree.untracked > 0)
    {
        blocked.push(action("commit", "changes are not staged yet", None, None));
    }

    if operation == Operation::None && !working_tree.clean {
        if let Some(upstream) = upstream {
            if upstream.behind > 0 {
                blocked.push(action(
                    "pull",
                    "working tree has local changes; clean or stage/commit before integrating upstream",
                    Some(&["git", "pull"]),
                    None,
                ));
            }
            if upstream.ahead > 0 && upstream.behind > 0 {
                blocked.push(action(
                    "integrate_diverged",
                    "working tree has local changes; clean or stage/commit before rebasing/merging upstream",
                    Some(&["git", "pull", "--rebase"]),
                    None,
                ));
            }
        }
    }

    blocked
}

fn push_resolve_if_conflicts(actions: &mut Vec<NextAction>, working_tree: &WorkingTree) {
    if working_tree.conflict_count > 0 {
        actions.push(action(
            "resolve_conflicts",
            "working tree has unmerged paths",
            None,
            None,
        ));
    }
}

/// continue/skip/abort 3종을 가진 sequencer류(rebase/am/cherry-pick/revert) 행동을 추가한다.
fn push_sequence_actions(
    actions: &mut Vec<NextAction>,
    kind_prefix: &str,
    command_op: &str,
    working_tree: &WorkingTree,
) {
    // continue는 충돌이 없을 때만 안전하다. `git <op> --continue`는 충돌 해결 전엔 실패한다.
    if working_tree.conflict_count == 0 {
        actions.push(action(
            &format!("{kind_prefix}_continue"),
            "resume after resolving",
            Some(&["git", command_op, "--continue"]),
            None,
        ));
    }
    actions.push(action(
        &format!("{kind_prefix}_skip"),
        "skip the current commit",
        Some(&["git", command_op, "--skip"]),
        None,
    ));
    actions.push(action(
        &format!("{kind_prefix}_abort"),
        "abort and restore the original state",
        Some(&["git", command_op, "--abort"]),
        Some("reversible"),
    ));
}

fn action(
    kind: &str,
    reason: &str,
    reference_command: Option<&[&str]>,
    risk: Option<&str>,
) -> NextAction {
    NextAction {
        kind: kind.to_string(),
        reason: reason.to_string(),
        reference_command: reference_command
            .map(|parts| parts.iter().map(|s| s.to_string()).collect()),
        risk: risk.map(|r| r.to_string()),
    }
}

pub(crate) fn detect_operation(git: &Git, path: &Path) -> Result<Operation> {
    let git_dir = absolute_git_dir(git, path)?;
    Ok(classify_operation(&git_dir))
}

/// git 디렉토리 안의 상태 파일로 진행 중인 작업을 판별한다.
/// cherry-pick/revert는 충돌 시 MERGE_HEAD를 함께 남길 수 있으므로 merge보다 먼저 확인한다.
fn classify_operation(git_dir: &Path) -> Operation {
    let has = |name: &str| git_dir.join(name).exists();

    if has("rebase-merge") {
        Operation::Rebasing
    } else if has("rebase-apply") {
        // rebase-apply/는 `git am`과 apply-backend `git rebase`가 공용으로 쓴다.
        // applying marker가 있으면 am 세션이다.
        if has("rebase-apply/applying") {
            Operation::Applying
        } else {
            Operation::Rebasing
        }
    } else if has("CHERRY_PICK_HEAD") {
        Operation::CherryPicking
    } else if has("REVERT_HEAD") {
        Operation::Reverting
    } else if let Some(op) = sequencer_operation(git_dir) {
        // multi-commit cherry-pick/revert에서 충돌 해결 후 직접 commit하면
        // CHERRY_PICK_HEAD/REVERT_HEAD가 사라지고 sequencer/todo만 남는다.
        op
    } else if has("MERGE_HEAD") {
        Operation::Merging
    } else if has("BISECT_LOG") {
        Operation::Bisecting
    } else {
        Operation::None
    }
}

/// sequencer/todo의 첫 명령으로 진행 중인 cherry-pick/revert를 판별한다.
/// rebase의 todo는 rebase-merge/에 따로 있으므로 여기엔 pick/revert만 나타난다.
fn sequencer_operation(git_dir: &Path) -> Option<Operation> {
    let todo = std::fs::read_to_string(git_dir.join("sequencer/todo")).ok()?;
    for line in todo.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        return match line.split_whitespace().next()? {
            "pick" | "p" => Some(Operation::CherryPicking),
            "revert" => Some(Operation::Reverting),
            _ => None,
        };
    }
    None
}

/// 입력 경로가 하위 디렉토리여도 저장소(워크트리) 루트의 절대경로를 반환한다.
/// worktree가 없는 bare 저장소 등에서는 입력 경로를 정규화해 fallback한다.
fn repo_root(git: &Git, path: &Path) -> Result<PathBuf> {
    let result = git.try_run_in(path, ["rev-parse", "--show-toplevel"])?;
    if result.success {
        let root = result.stdout.trim();
        if !root.is_empty() {
            return Ok(PathBuf::from(root));
        }
    }
    Ok(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// worktree/submodule에서도 정확한 git 디렉토리를 얻기 위해 git에게 직접 물어본다.
fn absolute_git_dir(git: &Git, path: &Path) -> Result<PathBuf> {
    git.run_path_in(path, ["rev-parse", "--absolute-git-dir"])
}

/// 현재 worktree가 family에서 어떤 위치인지 요약한다.
fn read_worktree_context(git: &Git, path: &Path) -> Result<WorktreeContext> {
    let worktrees = worktree::list_worktrees(path)?;

    // main 판정은 경로 비교 대신 git에게 맡긴다.
    // linked worktree는 git-dir이 .git/worktrees/<name>이라 공통 git-dir과 다르다.
    let git_dir = git.run_in(path, ["rev-parse", "--absolute-git-dir"])?;
    let common_dir = git.run_in(
        path,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let is_bare = git
        .try_run_in(path, ["rev-parse", "--is-bare-repository"])?
        .stdout
        .trim()
        == "true";

    let kind = classify_worktree_kind(is_bare, git_dir.stdout.trim(), common_dir.stdout.trim());

    // main worktree는 worktree list의 첫 항목이다. 단 첫 항목이 bare이면
    // (bare-primary family) main worktree 자체가 없으므로 None이다.
    let main = worktrees
        .first()
        .filter(|wt| !wt.bare)
        .map(|wt| wt.path.clone());

    let family_count = worktrees.len() as u32;
    let linked_count = family_count.saturating_sub(1);

    Ok(WorktreeContext {
        kind,
        main,
        family_count,
        linked_count,
    })
}

/// git-dir과 공통 git-dir 비교로 현재 worktree 종류를 판별한다.
fn classify_worktree_kind(is_bare: bool, git_dir: &str, common_dir: &str) -> WorktreeKind {
    if is_bare {
        WorktreeKind::Bare
    } else if git_dir == common_dir {
        WorktreeKind::Main
    } else {
        WorktreeKind::Linked
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// 가짜 git 디렉토리를 만들고 주어진 상태 파일/디렉토리를 채운다.
    fn git_dir_with(entries: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("create temp dir");
        for entry in entries {
            let target = dir.path().join(entry);
            // rebase-merge/rebase-apply는 디렉토리, 나머지는 파일로 만든다.
            if entry.starts_with("rebase-") {
                fs::create_dir(&target).expect("create dir");
            } else {
                fs::write(&target, "x").expect("write file");
            }
        }
        dir
    }

    #[test]
    fn classifies_none_when_clean() {
        let dir = git_dir_with(&[]);
        assert_eq!(classify_operation(dir.path()), Operation::None);
    }

    #[test]
    fn classifies_merging() {
        let dir = git_dir_with(&["MERGE_HEAD"]);
        assert_eq!(classify_operation(dir.path()), Operation::Merging);
    }

    #[test]
    fn classifies_rebasing_from_either_directory() {
        let merge_based = git_dir_with(&["rebase-merge"]);
        assert_eq!(classify_operation(merge_based.path()), Operation::Rebasing);

        let apply_based = git_dir_with(&["rebase-apply"]);
        assert_eq!(classify_operation(apply_based.path()), Operation::Rebasing);
    }

    #[test]
    fn cherry_pick_takes_priority_over_merge() {
        // cherry-pick 충돌은 CHERRY_PICK_HEAD와 MERGE_HEAD를 함께 남길 수 있다.
        let dir = git_dir_with(&["CHERRY_PICK_HEAD", "MERGE_HEAD"]);
        assert_eq!(classify_operation(dir.path()), Operation::CherryPicking);
    }

    #[test]
    fn revert_takes_priority_over_merge() {
        let dir = git_dir_with(&["REVERT_HEAD", "MERGE_HEAD"]);
        assert_eq!(classify_operation(dir.path()), Operation::Reverting);
    }

    #[test]
    fn classifies_bisecting() {
        let dir = git_dir_with(&["BISECT_LOG"]);
        assert_eq!(classify_operation(dir.path()), Operation::Bisecting);
    }

    #[test]
    fn classifies_am_session_via_applying_marker() {
        let dir = git_dir_with(&["rebase-apply"]);
        fs::write(dir.path().join("rebase-apply/applying"), "").expect("write");
        assert_eq!(classify_operation(dir.path()), Operation::Applying);
    }

    #[test]
    fn rebase_apply_without_applying_is_rebasing() {
        // apply-backend git rebase: rebase-apply는 있지만 applying은 없다.
        let dir = git_dir_with(&["rebase-apply"]);
        assert_eq!(classify_operation(dir.path()), Operation::Rebasing);
    }

    #[test]
    fn classifies_cherry_pick_from_sequencer_only() {
        // CHERRY_PICK_HEAD 없이 sequencer/todo만 남은 상태(충돌 해결 후 직접 commit).
        let dir = tempfile::tempdir().expect("create temp dir");
        fs::create_dir(dir.path().join("sequencer")).expect("create dir");
        fs::write(
            dir.path().join("sequencer/todo"),
            "pick abc123 A\npick def456 B\n",
        )
        .expect("write");
        assert_eq!(classify_operation(dir.path()), Operation::CherryPicking);
    }

    #[test]
    fn classifies_revert_from_sequencer_only() {
        let dir = tempfile::tempdir().expect("create temp dir");
        fs::create_dir(dir.path().join("sequencer")).expect("create dir");
        fs::write(dir.path().join("sequencer/todo"), "revert abc123 undo\n").expect("write");
        assert_eq!(classify_operation(dir.path()), Operation::Reverting);
    }

    #[test]
    fn sequencer_with_only_comments_is_none() {
        let dir = tempfile::tempdir().expect("create temp dir");
        fs::create_dir(dir.path().join("sequencer")).expect("create dir");
        fs::write(dir.path().join("sequencer/todo"), "# comment only\n\n").expect("write");
        assert_eq!(classify_operation(dir.path()), Operation::None);
    }

    #[test]
    fn parses_ahead_behind_counts() {
        // 출력 형식: "<behind>\t<ahead>"
        assert_eq!(parse_ahead_behind("0\t2\n"), Some((0, 2)));
        assert_eq!(parse_ahead_behind("3\t0\n"), Some((3, 0)));
        assert_eq!(parse_ahead_behind("1\t4"), Some((1, 4)));
    }

    #[test]
    fn parse_ahead_behind_rejects_garbage() {
        assert_eq!(parse_ahead_behind(""), None);
        assert_eq!(parse_ahead_behind("oops"), None);
        assert_eq!(parse_ahead_behind("1\t2 extra"), None);
    }

    #[test]
    fn warnings_mark_failed_upstream_comparison() {
        let upstream = Some(UpstreamInfo {
            name: "origin/main".to_string(),
            ahead: 0,
            behind: 0,
            comparison_basis: UpstreamComparisonBasis::LocalTrackingRef,
            comparison_status: UpstreamComparisonStatus::Failed,
        });

        let warnings = compute_warnings(&upstream);
        assert!(warnings
            .iter()
            .any(|w| w.code == "upstream_freshness_unknown"));
        assert!(warnings
            .iter()
            .any(|w| w.code == "upstream_comparison_failed"));
    }

    // Working-tree classification moved to git/status.rs (classify_porcelain_z),
    // which is unit-tested there with -z input including unicode/rename cases.

    fn clean_wt() -> WorkingTree {
        WorkingTree {
            clean: true,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflict_count: 0,
            conflicts: vec![],
        }
    }

    fn kinds(actions: &[NextAction]) -> Vec<&str> {
        actions.iter().map(|a| a.kind.as_str()).collect()
    }

    fn assert_actions_are_cataloged(next: &NextGuardrails) {
        let catalog: std::collections::BTreeSet<&str> =
            EVALUATED_INSPECT_ACTIONS.iter().copied().collect();

        for action in next
            .allowed
            .iter()
            .chain(next.blocked.iter())
            .chain(next.needs_human_review.iter())
        {
            assert!(
                catalog.contains(action.kind.as_str()),
                "missing action kind in EVALUATED_INSPECT_ACTIONS: {}",
                action.kind
            );
        }
    }

    #[test]
    fn next_clean_without_upstream_is_empty() {
        let next = compute_next_guardrails(Operation::None, &clean_wt(), &None);
        assert!(next.allowed.is_empty());
        assert!(next.blocked.is_empty());
        assert!(next.needs_human_review.is_empty());
    }

    #[test]
    fn emitted_next_actions_are_all_in_the_inspect_action_catalog() {
        let staged_wt = WorkingTree {
            clean: false,
            staged: 1,
            unstaged: 1,
            untracked: 1,
            conflict_count: 0,
            conflicts: vec![],
        };
        let conflict_wt = WorkingTree {
            clean: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflict_count: 1,
            conflicts: vec!["f.txt".to_string()],
        };
        let behind = Some(UpstreamInfo {
            name: "origin/main".to_string(),
            ahead: 0,
            behind: 1,
            comparison_basis: UpstreamComparisonBasis::LocalTrackingRef,
            comparison_status: UpstreamComparisonStatus::Ok,
        });
        let diverged = Some(UpstreamInfo {
            name: "origin/main".to_string(),
            ahead: 1,
            behind: 1,
            comparison_basis: UpstreamComparisonBasis::LocalTrackingRef,
            comparison_status: UpstreamComparisonStatus::Ok,
        });

        let cases = [
            compute_next_guardrails(Operation::None, &staged_wt, &behind),
            compute_next_guardrails(Operation::None, &clean_wt(), &diverged),
            compute_next_guardrails(Operation::Merging, &conflict_wt, &None),
            compute_next_guardrails(Operation::Merging, &staged_wt, &None),
            compute_next_guardrails(Operation::Rebasing, &conflict_wt, &None),
            compute_next_guardrails(Operation::Rebasing, &clean_wt(), &None),
            compute_next_guardrails(Operation::Applying, &clean_wt(), &None),
            compute_next_guardrails(Operation::CherryPicking, &clean_wt(), &None),
            compute_next_guardrails(Operation::Reverting, &clean_wt(), &None),
            compute_next_guardrails(Operation::Bisecting, &clean_wt(), &None),
        ];

        for next in &cases {
            assert_actions_are_cataloged(next);
        }
    }

    #[test]
    fn next_allows_commit_when_staged() {
        let wt = WorkingTree {
            clean: false,
            staged: 1,
            unstaged: 0,
            untracked: 0,
            conflict_count: 0,
            conflicts: vec![],
        };
        let next = compute_next_guardrails(Operation::None, &wt, &None);
        assert!(kinds(&next.allowed).contains(&"commit"));
        assert!(!kinds(&next.blocked).contains(&"commit"));
    }

    #[test]
    fn next_allows_push_pull_diverged_when_clean() {
        let ahead = Some(UpstreamInfo {
            name: "origin/main".to_string(),
            ahead: 2,
            behind: 0,
            comparison_basis: UpstreamComparisonBasis::LocalTrackingRef,
            comparison_status: UpstreamComparisonStatus::Ok,
        });
        assert!(
            kinds(&compute_next_guardrails(Operation::None, &clean_wt(), &ahead).allowed)
                .contains(&"push")
        );

        let behind = Some(UpstreamInfo {
            name: "origin/main".to_string(),
            ahead: 0,
            behind: 3,
            comparison_basis: UpstreamComparisonBasis::LocalTrackingRef,
            comparison_status: UpstreamComparisonStatus::Ok,
        });
        assert!(
            kinds(&compute_next_guardrails(Operation::None, &clean_wt(), &behind).allowed)
                .contains(&"pull")
        );

        let diverged = Some(UpstreamInfo {
            name: "origin/main".to_string(),
            ahead: 1,
            behind: 1,
            comparison_basis: UpstreamComparisonBasis::LocalTrackingRef,
            comparison_status: UpstreamComparisonStatus::Ok,
        });
        assert!(
            kinds(&compute_next_guardrails(Operation::None, &clean_wt(), &diverged).allowed)
                .contains(&"integrate_diverged")
        );
    }

    #[test]
    fn next_rebasing_has_continue_skip_abort_only() {
        let next = compute_next_guardrails(Operation::Rebasing, &clean_wt(), &None);
        let ks = kinds(&next.allowed);
        assert!(ks.contains(&"rebase_continue"));
        assert!(ks.contains(&"rebase_skip"));
        assert!(ks.contains(&"rebase_abort"));
        // 진행 중 작업이 있으면 일반 흐름(commit/push)은 제안하지 않는다.
        assert!(!ks.contains(&"commit"));
        assert!(!ks.contains(&"push"));
    }

    #[test]
    fn next_merging_with_conflicts_marks_abort_reversible() {
        let wt = WorkingTree {
            clean: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflict_count: 1,
            conflicts: vec!["f.txt".to_string()],
        };
        let next = compute_next_guardrails(Operation::Merging, &wt, &None);
        let ks = kinds(&next.allowed);
        assert!(ks.contains(&"resolve_conflicts"));
        assert!(ks.contains(&"merge_abort"));
        assert!(kinds(&next.blocked).contains(&"continue_operation"));

        let abort = next
            .allowed
            .iter()
            .find(|a| a.kind == "merge_abort")
            .unwrap();
        assert_eq!(abort.risk.as_deref(), Some("reversible"));
        assert_eq!(
            abort.reference_command.as_deref(),
            Some(["git", "merge", "--abort"].map(String::from).as_slice())
        );
    }

    #[test]
    fn next_merge_resolved_suggests_continue() {
        // 충돌 해결(add)했지만 아직 commit 전: merging + conflict 0 + staged.
        let wt = WorkingTree {
            clean: false,
            staged: 1,
            unstaged: 0,
            untracked: 0,
            conflict_count: 0,
            conflicts: vec![],
        };
        let next = compute_next_guardrails(Operation::Merging, &wt, &None);
        let ks = kinds(&next.allowed);
        assert!(ks.contains(&"merge_continue"));
        assert!(ks.contains(&"merge_abort"));
        assert!(!ks.contains(&"resolve_conflicts"));
        assert!(!kinds(&next.blocked).contains(&"continue_operation"));
    }

    #[test]
    fn next_untracked_only_suggests_stage_and_blocks_commit() {
        let wt = WorkingTree {
            clean: false,
            staged: 0,
            unstaged: 0,
            untracked: 1,
            conflict_count: 0,
            conflicts: vec![],
        };
        let next = compute_next_guardrails(Operation::None, &wt, &None);
        let ks = kinds(&next.allowed);
        // untracked는 아직 commit 대상이 아니므로 stage_changes를 제안한다.
        assert!(ks.contains(&"stage_changes"));
        assert!(!ks.contains(&"commit"));
        assert!(kinds(&next.blocked).contains(&"commit"));
    }

    #[test]
    fn next_dirty_suppresses_pull_and_blocks_it() {
        // dirty + behind: pull은 워킹 트리를 건드리니 억제하고 stage/commit을 먼저 유도한다.
        let wt = WorkingTree {
            clean: false,
            staged: 0,
            unstaged: 1,
            untracked: 0,
            conflict_count: 0,
            conflicts: vec![],
        };
        let behind = Some(UpstreamInfo {
            name: "origin/main".to_string(),
            ahead: 0,
            behind: 2,
            comparison_basis: UpstreamComparisonBasis::LocalTrackingRef,
            comparison_status: UpstreamComparisonStatus::Ok,
        });
        let next = compute_next_guardrails(Operation::None, &wt, &behind);
        let ks = kinds(&next.allowed);
        assert!(ks.contains(&"stage_changes"));
        assert!(!ks.contains(&"pull"));
        assert!(kinds(&next.blocked).contains(&"pull"));
    }

    #[test]
    fn next_rebase_conflict_hides_continue_and_blocks_it() {
        let wt = WorkingTree {
            clean: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflict_count: 1,
            conflicts: vec!["f.txt".to_string()],
        };
        let next = compute_next_guardrails(Operation::Rebasing, &wt, &None);
        let ks = kinds(&next.allowed);
        assert!(ks.contains(&"resolve_conflicts"));
        assert!(ks.contains(&"rebase_skip"));
        assert!(ks.contains(&"rebase_abort"));
        // 충돌 중에는 continue를 제안하지 않는다.
        assert!(!ks.contains(&"rebase_continue"));
        assert!(kinds(&next.blocked).contains(&"continue_operation"));
    }

    #[test]
    fn worktree_kind_main_when_git_dirs_match() {
        assert_eq!(
            classify_worktree_kind(false, "/repo/.git", "/repo/.git"),
            WorktreeKind::Main
        );
    }

    #[test]
    fn worktree_kind_linked_when_git_dirs_differ() {
        assert_eq!(
            classify_worktree_kind(false, "/repo/.git/worktrees/wt", "/repo/.git"),
            WorktreeKind::Linked
        );
    }

    #[test]
    fn worktree_kind_bare_regardless_of_dirs() {
        assert_eq!(
            classify_worktree_kind(true, "/repo.git", "/repo.git"),
            WorktreeKind::Bare
        );
    }
}
