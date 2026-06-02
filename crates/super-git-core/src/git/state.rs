use std::path::{Path, PathBuf};

use crate::git::command::Git;
use crate::model::{HeadInfo, Operation, RepoState};
use crate::Result;

/// 저장소의 HEAD 위치와 진행 중인 작업을 한 번에 읽는다.
pub fn read_state(path: &Path) -> Result<RepoState> {
    let git = Git::default();
    let head = read_head(&git, path)?;
    let operation = detect_operation(&git, path)?;
    Ok(RepoState { head, operation })
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

fn detect_operation(git: &Git, path: &Path) -> Result<Operation> {
    let git_dir = absolute_git_dir(git, path)?;
    Ok(classify_operation(&git_dir))
}

/// git 디렉토리 안의 상태 파일로 진행 중인 작업을 판별한다.
/// cherry-pick/revert는 충돌 시 MERGE_HEAD를 함께 남길 수 있으므로 merge보다 먼저 확인한다.
fn classify_operation(git_dir: &Path) -> Operation {
    let has = |name: &str| git_dir.join(name).exists();

    if has("rebase-merge") || has("rebase-apply") {
        Operation::Rebasing
    } else if has("CHERRY_PICK_HEAD") {
        Operation::CherryPicking
    } else if has("REVERT_HEAD") {
        Operation::Reverting
    } else if has("MERGE_HEAD") {
        Operation::Merging
    } else if has("BISECT_LOG") {
        Operation::Bisecting
    } else {
        Operation::None
    }
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
}
