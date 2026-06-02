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
";

        let worktrees = parse_worktree_list(output);

        assert_eq!(worktrees.len(), 3);
        assert_eq!(worktrees[0].path, PathBuf::from("/repo"));
        assert_eq!(worktrees[0].branch, Some("main".to_string()));
        assert!(!worktrees[0].detached);
        assert_eq!(
            worktrees[1].head,
            Some("2222222222222222222222222222222222222222".to_string())
        );
        assert!(worktrees[1].detached);
        assert!(worktrees[2].bare);
    }
}
