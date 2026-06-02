use std::path::{Path, PathBuf};

use crate::git::command::Git;
use crate::model::{HeadInfo, Operation, RepoState, UpstreamInfo, WorkingTree};
use crate::Result;

/// 저장소의 HEAD 위치와 진행 중인 작업을 한 번에 읽는다.
pub fn read_state(path: &Path) -> Result<RepoState> {
    let git = Git::default();
    let root = repo_root(&git, path)?;
    let head = read_head(&git, path)?;
    let upstream = read_upstream(&git, path)?;
    let working_tree = read_working_tree(&git, path)?;
    let operation = detect_operation(&git, path)?;
    Ok(RepoState {
        root,
        head,
        upstream,
        working_tree,
        operation,
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
    let (behind, ahead) = parse_ahead_behind(&counts.stdout);

    Ok(Some(UpstreamInfo {
        name,
        ahead,
        behind,
    }))
}

/// `rev-list --count --left-right @{upstream}...HEAD` 출력("<behind>\t<ahead>")을 파싱한다.
fn parse_ahead_behind(output: &str) -> (u32, u32) {
    let mut parts = output.split_whitespace();
    let behind = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ahead = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (behind, ahead)
}

/// 워킹 트리 변경을 요약한다. 상세 목록은 status 명령에 맡기고 카운트+충돌 목록만 만든다.
fn read_working_tree(git: &Git, path: &Path) -> Result<WorkingTree> {
    let output = git.run_in(path, ["status", "--porcelain=v1"])?;
    Ok(classify_working_tree(&output.stdout))
}

/// `git status --porcelain=v1` 출력을 staged/unstaged/untracked 카운트와 충돌 목록으로 분류한다.
fn classify_working_tree(output: &str) -> WorkingTree {
    let mut staged = 0;
    let mut unstaged = 0;
    let mut untracked = 0;
    let mut conflicts = Vec::new();

    for line in output.lines() {
        // 각 라인은 "XY <path>" 형태다(X=index, Y=worktree). 너무 짧으면 건너뛴다.
        if line.len() < 4 {
            continue;
        }
        let code = &line[..2];
        let path = line[3..].to_string();
        let bytes = code.as_bytes();
        let (x, y) = (bytes[0] as char, bytes[1] as char);

        if code == "??" {
            untracked += 1;
        } else if is_conflict(x, y) {
            conflicts.push(path);
        } else {
            if is_change(x) {
                staged += 1;
            }
            if is_change(y) {
                unstaged += 1;
            }
        }
    }

    let conflict_count = conflicts.len() as u32;
    let clean = staged == 0 && unstaged == 0 && untracked == 0 && conflict_count == 0;

    WorkingTree {
        clean,
        staged,
        unstaged,
        untracked,
        conflict_count,
        conflicts,
    }
}

/// unmerged(충돌) 상태 코드인지 판별한다.
/// DD/AA, 그리고 X나 Y가 U인 모든 조합(AU/UD/UA/DU/UU)이 충돌이다.
fn is_conflict(x: char, y: char) -> bool {
    x == 'U' || y == 'U' || (x == 'D' && y == 'D') || (x == 'A' && y == 'A')
}

/// 변경을 나타내는 상태 문자인지 판별한다(공백=무변경, ?=미추적 제외).
fn is_change(code: char) -> bool {
    code != ' ' && code != '?'
}

fn detect_operation(git: &Git, path: &Path) -> Result<Operation> {
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
    let output = git.run_in(path, ["rev-parse", "--absolute-git-dir"])?;
    Ok(PathBuf::from(output.stdout.trim()))
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
        assert_eq!(parse_ahead_behind("0\t2\n"), (0, 2));
        assert_eq!(parse_ahead_behind("3\t0\n"), (3, 0));
        assert_eq!(parse_ahead_behind("1\t4"), (1, 4));
    }

    #[test]
    fn parse_ahead_behind_tolerates_garbage() {
        assert_eq!(parse_ahead_behind(""), (0, 0));
        assert_eq!(parse_ahead_behind("oops"), (0, 0));
    }

    #[test]
    fn classify_working_tree_clean_when_empty() {
        let wt = classify_working_tree("");
        assert!(wt.clean);
        assert_eq!(wt.staged, 0);
        assert_eq!(wt.unstaged, 0);
        assert_eq!(wt.untracked, 0);
        assert_eq!(wt.conflict_count, 0);
        assert!(wt.conflicts.is_empty());
    }

    #[test]
    fn classify_working_tree_counts_changes() {
        // "M  a" staged, " M b" unstaged, "MM c" both, "?? d" untracked
        let output = "M  a.txt\n M b.txt\nMM c.txt\n?? d.txt\n";
        let wt = classify_working_tree(output);
        assert!(!wt.clean);
        assert_eq!(wt.staged, 2); // a.txt, c.txt
        assert_eq!(wt.unstaged, 2); // b.txt, c.txt
        assert_eq!(wt.untracked, 1); // d.txt
        assert_eq!(wt.conflict_count, 0);
    }

    #[test]
    fn classify_working_tree_collects_conflicts() {
        let output = "UU both_mod.txt\nAA both_add.txt\nDU del_by_us.txt\n";
        let wt = classify_working_tree(output);
        assert_eq!(wt.conflict_count, 3);
        assert_eq!(
            wt.conflicts,
            vec!["both_mod.txt", "both_add.txt", "del_by_us.txt"]
        );
        // 충돌은 staged/unstaged 카운트에서 제외된다.
        assert_eq!(wt.staged, 0);
        assert_eq!(wt.unstaged, 0);
        assert!(!wt.clean);
    }
}
