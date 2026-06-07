use std::path::{Path, PathBuf};

use crate::git::command::Git;
use crate::model::WorktreeInfo;
use crate::Result;

pub fn list_worktrees(path: &Path) -> Result<Vec<WorktreeInfo>> {
    let output = Git::default().run_in(path, ["worktree", "list", "--porcelain"])?;
    Ok(parse_worktree_list(&output.stdout))
}

pub fn parse_worktree_list(output: &str) -> Vec<WorktreeInfo> {
    let mut worktrees = Vec::new();
    let mut current: Option<WorktreeInfo> = None;

    for line in output.lines() {
        if line.trim().is_empty() {
            push_current(&mut worktrees, &mut current);
            continue;
        }

        if let Some(path) = line.strip_prefix("worktree ") {
            push_current(&mut worktrees, &mut current);
            current = Some(WorktreeInfo::new(PathBuf::from(path)));
            continue;
        }

        let Some(worktree) = current.as_mut() else {
            continue;
        };

        if let Some(head) = line.strip_prefix("HEAD ") {
            worktree.head = Some(head.to_string());
        } else if let Some(branch) = line.strip_prefix("branch ") {
            worktree.branch = Some(short_branch_name(branch));
        } else if line == "detached" {
            worktree.detached = true;
        } else if line == "bare" {
            worktree.bare = true;
        } else if line == "locked" || line.starts_with("locked ") {
            worktree.locked = true;
        } else if line == "prunable" || line.starts_with("prunable ") {
            worktree.prunable = true;
        }
    }

    push_current(&mut worktrees, &mut current);
    worktrees
}

fn push_current(worktrees: &mut Vec<WorktreeInfo>, current: &mut Option<WorktreeInfo>) {
    if let Some(worktree) = current.take() {
        worktrees.push(worktree);
    }
}

fn short_branch_name(branch: &str) -> String {
    branch
        .strip_prefix("refs/heads/")
        .unwrap_or(branch)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_worktree_output() {
        let output = "\
worktree /repo
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main

worktree /repo-feature
HEAD 2222222222222222222222222222222222222222
detached

worktree /bare
bare

worktree /locked
HEAD 3333333333333333333333333333333333333333
branch refs/heads/locked
locked manual reason

worktree /prunable
HEAD 4444444444444444444444444444444444444444
branch refs/heads/prunable
prunable gitdir file points to non-existent location
";

        let worktrees = parse_worktree_list(output);

        assert_eq!(worktrees.len(), 5);
        assert_eq!(worktrees[0].path, PathBuf::from("/repo"));
        assert_eq!(worktrees[0].branch, Some("main".to_string()));
        assert!(!worktrees[0].detached);
        assert!(!worktrees[0].locked);
        assert!(!worktrees[0].prunable);
        assert_eq!(
            worktrees[1].head,
            Some("2222222222222222222222222222222222222222".to_string())
        );
        assert!(worktrees[1].detached);
        assert!(worktrees[2].bare);
        assert!(worktrees[3].locked);
        assert!(!worktrees[3].prunable);
        assert!(!worktrees[4].locked);
        assert!(worktrees[4].prunable);
    }
}
