use std::path::Path;

use crate::git::command::{self, Git};
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
/// blank line in the line-based form). The `worktree` path is decoded from raw
/// bytes so a non-UTF-8 path survives; the other attributes are ASCII.
pub fn parse_worktree_list(stdout: &[u8]) -> Vec<WorktreeInfo> {
    let mut worktrees = Vec::new();
    let mut current: Option<WorktreeInfo> = None;

    for record in stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            push_current(&mut worktrees, &mut current);
            continue;
        }

        if let Some(path) = record.strip_prefix(b"worktree ") {
            push_current(&mut worktrees, &mut current);
            current = Some(WorktreeInfo::new(command::os_path_from_bytes(path)));
            continue;
        }

        let Some(worktree) = current.as_mut() else {
            continue;
        };

        // HEAD/branch/detached/bare/locked/prunable are all ASCII tokens.
        let attribute = String::from_utf8_lossy(record);
        if let Some(head) = attribute.strip_prefix("HEAD ") {
            worktree.head = Some(head.to_string());
        } else if let Some(branch) = attribute.strip_prefix("branch ") {
            worktree.branch = Some(short_branch_name(branch));
        } else if attribute == "detached" {
            worktree.detached = true;
        } else if attribute == "bare" {
            worktree.bare = true;
        } else if attribute == "locked" || attribute.starts_with("locked ") {
            worktree.locked = true;
        } else if attribute == "prunable" || attribute.starts_with("prunable ") {
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
    use std::path::PathBuf;

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

    #[cfg(unix)]
    #[test]
    fn parses_worktree_path_with_non_utf8_bytes() {
        use std::os::unix::ffi::OsStrExt;

        // A latin-1 0xe9 byte in the path is invalid UTF-8; lossy decoding would
        // mangle it to U+FFFD, so the parsed path would not match on disk.
        let mut output = b"worktree /weird/caf".to_vec();
        output.push(0xe9);
        output.extend_from_slice(
            b"\0HEAD 1111111111111111111111111111111111111111\0branch refs/heads/main\0\0",
        );

        let worktrees = parse_worktree_list(&output);

        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].path.as_os_str().as_bytes(), b"/weird/caf\xe9");
        assert_eq!(worktrees[0].branch, Some("main".to_string()));
    }
}
