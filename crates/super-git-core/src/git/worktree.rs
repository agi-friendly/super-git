use std::path::{Path, PathBuf};

use crate::git::command::Git;
use crate::model::WorktreeInfo;
use crate::Result;

pub fn list_worktrees(path: &Path) -> Result<Vec<WorktreeInfo>> {
    // -z so a worktree path containing a newline parses as one record; the
    // line-based form would truncate it and misattribute the remainder.
    let output = Git::default().run_bytes_in(path, ["worktree", "list", "--porcelain", "-z"])?;
    Ok(parse_worktree_list(&output.stdout))
}

/// Parse `git worktree list --porcelain -z`: each attribute is NUL-terminated
/// and an empty record separates worktree entries (the `-z` analogue of the
/// blank line in the line-based form).
pub fn parse_worktree_list(stdout: &[u8]) -> Vec<WorktreeInfo> {
    let text = String::from_utf8_lossy(stdout);
    let mut worktrees = Vec::new();
    let mut current: Option<WorktreeInfo> = None;

    for record in text.split('\0') {
        if record.is_empty() {
            push_current(&mut worktrees, &mut current);
            continue;
        }

        if let Some(path) = record.strip_prefix("worktree ") {
            push_current(&mut worktrees, &mut current);
            current = Some(WorktreeInfo::new(PathBuf::from(path)));
            continue;
        }

        let Some(worktree) = current.as_mut() else {
            continue;
        };

        if let Some(head) = record.strip_prefix("HEAD ") {
            worktree.head = Some(head.to_string());
        } else if let Some(branch) = record.strip_prefix("branch ") {
            worktree.branch = Some(short_branch_name(branch));
        } else if record == "detached" {
            worktree.detached = true;
        } else if record == "bare" {
            worktree.bare = true;
        } else if record == "locked" || record.starts_with("locked ") {
            worktree.locked = true;
        } else if record == "prunable" || record.starts_with("prunable ") {
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
        // In `-z` output each attribute is NUL-terminated and entries are
        // separated by an empty record, i.e. the line form with '\n' -> '\0'.
        let output = output.replace('\n', "\0");

        let worktrees = parse_worktree_list(output.as_bytes());

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

    #[test]
    fn parses_worktree_path_containing_newline() {
        // A POSIX-legal newline in the path: the line-based parser would split
        // the entry, but the -z record keeps it whole.
        let output = "worktree /weird\nname\0\
HEAD 1111111111111111111111111111111111111111\0\
branch refs/heads/main\0\0";

        let worktrees = parse_worktree_list(output.as_bytes());

        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].path, PathBuf::from("/weird\nname"));
        assert_eq!(worktrees[0].branch, Some("main".to_string()));
    }
}
