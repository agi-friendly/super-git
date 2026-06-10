use std::path::Path;

use crate::git::command::Git;
use crate::model::StatusOutput;
use crate::Result;

pub fn read_status(path: &Path) -> Result<StatusOutput> {
    let output = Git::default().run_in(path, ["status", "--porcelain=v1", "--branch"])?;
    Ok(parse_status_porcelain(&output.stdout))
}

/// Working-tree counts and raw conflict paths from `git status --porcelain=v1
/// -z`. The `-z` form keeps paths raw (no `core.quotePath` C-quoting) and lets
/// a path containing a newline parse correctly, unlike the line-based form.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct PorcelainCounts {
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub ignored: u32,
    pub conflicts: Vec<String>,
}

impl PorcelainCounts {
    pub(crate) fn conflict_count(&self) -> u32 {
        self.conflicts.len() as u32
    }
}

/// Parse NUL-delimited porcelain v1 status. Each record is `XY<space><path>`.
/// Rename/copy entries carry a second NUL-terminated original path which is
/// consumed (the entry already counts as one staged change), not re-parsed.
pub(crate) fn classify_porcelain_z(stdout: &[u8]) -> PorcelainCounts {
    let mut counts = PorcelainCounts::default();
    let mut records = stdout.split(|byte| *byte == 0).filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        if record.len() < 3 {
            continue;
        }
        let (x, y) = (record[0] as char, record[1] as char);
        // record[2] is the separator space; the rest is the raw path.
        let path = String::from_utf8_lossy(&record[3..]).into_owned();
        match &record[..2] {
            b"!!" => counts.ignored += 1,
            b"??" => counts.untracked += 1,
            _ if is_conflict(x, y) => counts.conflicts.push(path),
            _ => {
                if is_change(x) {
                    counts.staged += 1;
                }
                if is_change(y) {
                    counts.unstaged += 1;
                }
            }
        }
        // A rename (R) or copy (C) in the index column adds a second record with
        // the original path; skip it so it is not treated as its own entry.
        if x == 'R' || x == 'C' {
            records.next();
        }
    }
    counts
}

/// Unmerged (conflict) status codes: DD/AA, or either column being `U`.
pub(crate) fn is_conflict(x: char, y: char) -> bool {
    x == 'U' || y == 'U' || (x == 'D' && y == 'D') || (x == 'A' && y == 'A')
}

/// A change marker (space = unchanged, `?` = untracked are excluded).
pub(crate) fn is_change(code: char) -> bool {
    code != ' ' && code != '?'
}

pub fn parse_status_porcelain(output: &str) -> StatusOutput {
    let mut branch_header = None;
    let mut entries = Vec::new();

    for line in output.lines() {
        if line.starts_with("## ") {
            branch_header = Some(line.to_string());
        } else if !line.trim().is_empty() {
            entries.push(line.to_string());
        }
    }

    StatusOutput {
        branch_header,
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_and_changed_entries() {
        let output = "## main...origin/main\n M README.md\n?? docs/roadmap.md\n";

        let status = parse_status_porcelain(output);

        assert_eq!(
            status.branch_header,
            Some("## main...origin/main".to_string())
        );
        assert_eq!(status.entries, vec![" M README.md", "?? docs/roadmap.md"]);
        assert!(!status.is_clean());
    }

    #[test]
    fn clean_status_has_no_entries() {
        let output = "## main\n";

        let status = parse_status_porcelain(output);

        assert_eq!(status.branch_header, Some("## main".to_string()));
        assert!(status.is_clean());
    }

    #[test]
    fn classify_porcelain_z_empty_is_clean() {
        let counts = classify_porcelain_z(b"");
        assert_eq!(counts, PorcelainCounts::default());
        assert_eq!(counts.conflict_count(), 0);
    }

    #[test]
    fn classify_porcelain_z_counts_changes_untracked_and_ignored() {
        // NUL-delimited: staged add, unstaged modify, untracked, ignored.
        let stdout = b"A  staged.txt\0 M tracked.txt\0?? new.txt\0!! build.log\0";
        let counts = classify_porcelain_z(stdout);
        assert_eq!(counts.staged, 1);
        assert_eq!(counts.unstaged, 1);
        assert_eq!(counts.untracked, 1);
        assert_eq!(counts.ignored, 1);
        assert_eq!(counts.conflict_count(), 0);
    }

    #[test]
    fn classify_porcelain_z_keeps_conflict_paths_raw() {
        // With -z, a unicode path is raw bytes, not C-quoted as it would be in
        // the line-based form (`UU "caf\303\251.txt"`).
        let stdout = "UU café.txt\0".as_bytes();
        let counts = classify_porcelain_z(stdout);
        assert_eq!(counts.conflict_count(), 1);
        assert_eq!(counts.conflicts, vec!["café.txt".to_string()]);
    }

    #[test]
    fn classify_porcelain_z_skips_rename_original_path() {
        // A rename adds a second NUL record (the original path); it must not be
        // miscounted as its own entry.
        let stdout = b"R  new name.txt\0old name.txt\0 M other.txt\0";
        let counts = classify_porcelain_z(stdout);
        assert_eq!(counts.staged, 1, "the rename is one staged change");
        assert_eq!(counts.unstaged, 1, "only other.txt is unstaged");
        assert_eq!(counts.untracked, 0);
    }

    #[test]
    fn classify_porcelain_z_handles_path_with_newline() {
        // The line-based parser would split this entry; -z keeps it whole.
        let stdout = b" M weird\nname.txt\0";
        let counts = classify_porcelain_z(stdout);
        assert_eq!(counts.unstaged, 1);
        assert_eq!(counts.untracked, 0);
    }
}
