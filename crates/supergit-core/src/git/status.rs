use std::path::Path;

use crate::git::command::Git;
use crate::model::StatusOutput;
use crate::Result;

pub fn read_status(path: &Path) -> Result<StatusOutput> {
    let output = Git::default().run_in(path, ["status", "--porcelain=v1", "--branch"])?;
    Ok(parse_status_porcelain(&output.stdout))
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
}
