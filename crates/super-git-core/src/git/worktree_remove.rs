use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::config::store::repository_id;
use crate::git::command::Git;
use crate::git::{state, worktree};
use crate::model::Operation;
use crate::{Result, SuperGitError};

const ACTION_WORKTREE_REMOVE: &str = "worktree_remove";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeRemoveScan {
    pub repository: WorktreeRemoveRepository,
    pub target: WorktreeRemoveTarget,
    pub blocks: Vec<WorktreeRemoveBlock>,
    pub execution_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeRemoveRepository {
    pub family_id: String,
    pub git_common_dir: PathBuf,
    pub main_worktree: Option<PathBuf>,
    pub selected_from: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeRemoveTarget {
    pub input_path: PathBuf,
    pub canonical_path: PathBuf,
    pub worktree_list_path: PathBuf,
    pub list_index: usize,
    pub kind: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub locked: bool,
    pub prunable: bool,
    pub is_current_worktree: bool,
    pub worktree_git_dir: Option<PathBuf>,
    pub git_common_dir: Option<PathBuf>,
    pub operation: Operation,
    pub working_tree: WorktreeRemoveWorkingTree,
    pub has_submodules: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeRemoveWorkingTree {
    pub clean: bool,
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub ignored: u32,
    pub conflict_count: u32,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeRemoveBlock {
    pub code: String,
    pub severity: String,
}

pub fn scan_worktree_remove_target(
    current_path: &Path,
    exact_target_path: &Path,
) -> Result<WorktreeRemoveScan> {
    validate_exact_absolute_path(exact_target_path)?;

    let git = Git::default();
    let (worktrees, list_source) = list_worktrees_for_scan(current_path, exact_target_path)?;
    let target_index = worktrees
        .iter()
        .position(|worktree| worktree.path == exact_target_path)
        .ok_or_else(|| SuperGitError::PreviewPreconditionFailed {
            action: ACTION_WORKTREE_REMOVE.to_string(),
            code: "target_path_not_exact_worktree_list_entry".to_string(),
            message: "target path must exactly match one git worktree list entry".to_string(),
        })?;
    let target_info = &worktrees[target_index];
    let repository_git_common_dir = git_common_dir(&git, &list_source)?;
    let current_worktree_root = worktree_root(&git, current_path).ok();
    let is_current_worktree = current_worktree_root
        .as_ref()
        .is_some_and(|root| root == &target_info.path);

    let target_git_dir =
        read_path_from_git(&git, &target_info.path, ["rev-parse", "--absolute-git-dir"]);
    let target_common_dir = read_path_from_git(
        &git,
        &target_info.path,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let operation = if can_scan_target_details(target_info) {
        state::detect_operation(&git, &target_info.path).unwrap_or(Operation::None)
    } else {
        Operation::None
    };
    let working_tree = if can_scan_target_details(target_info) {
        read_working_tree_with_ignored(&git, &target_info.path)?
    } else {
        WorktreeRemoveWorkingTree::clean()
    };
    let has_submodules = if can_scan_target_details(target_info) {
        has_submodules(&git, &target_info.path)?
    } else {
        false
    };
    let canonical_path = std::fs::canonicalize(exact_target_path)
        .unwrap_or_else(|_| exact_target_path.to_path_buf());
    let target = WorktreeRemoveTarget {
        input_path: exact_target_path.to_path_buf(),
        canonical_path,
        worktree_list_path: target_info.path.clone(),
        list_index: target_index,
        kind: target_kind(target_index, target_info).to_string(),
        head: target_info.head.clone(),
        branch: target_info.branch.clone(),
        detached: target_info.detached,
        locked: target_info.locked,
        prunable: target_info.prunable,
        is_current_worktree,
        worktree_git_dir: target_git_dir,
        git_common_dir: target_common_dir,
        operation,
        working_tree,
        has_submodules,
    };
    let blocks = collect_blocks(&target, &repository_git_common_dir);
    let execution_status = if blocks.is_empty() {
        "preview_only"
    } else {
        "blocked"
    }
    .to_string();

    Ok(WorktreeRemoveScan {
        repository: WorktreeRemoveRepository {
            family_id: repository_id(&repository_git_common_dir),
            git_common_dir: repository_git_common_dir,
            main_worktree: worktrees
                .first()
                .filter(|worktree| !worktree.bare)
                .map(|wt| wt.path.clone()),
            selected_from: list_source,
        },
        target,
        blocks,
        execution_status,
    })
}

fn validate_exact_absolute_path(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(SuperGitError::PreviewPreconditionFailed {
            action: ACTION_WORKTREE_REMOVE.to_string(),
            code: "target_path_not_absolute".to_string(),
            message: "worktree remove preview requires an absolute target path".to_string(),
        });
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(SuperGitError::PreviewPreconditionFailed {
            action: ACTION_WORKTREE_REMOVE.to_string(),
            code: "target_path_not_clean".to_string(),
            message: "target path must not contain current or parent directory components"
                .to_string(),
        });
    }
    Ok(())
}

fn list_worktrees_for_scan(
    current_path: &Path,
    exact_target_path: &Path,
) -> Result<(Vec<crate::model::WorktreeInfo>, PathBuf)> {
    match worktree::list_worktrees(exact_target_path) {
        Ok(worktrees) => Ok((worktrees, exact_target_path.to_path_buf())),
        Err(target_error) => match worktree::list_worktrees(current_path) {
            Ok(worktrees) => Ok((worktrees, current_path.to_path_buf())),
            Err(_) => Err(target_error),
        },
    }
}

fn git_common_dir(git: &Git, path: &Path) -> Result<PathBuf> {
    let output = git.run_in(
        path,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    Ok(PathBuf::from(output.stdout.trim()))
}

fn worktree_root(git: &Git, path: &Path) -> Result<PathBuf> {
    let output = git.run_in(path, ["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(output.stdout.trim()))
}

fn read_path_from_git<const N: usize>(git: &Git, path: &Path, args: [&str; N]) -> Option<PathBuf> {
    git.try_run_in(path, args)
        .ok()
        .filter(|output| output.success)
        .map(|output| PathBuf::from(output.stdout.trim()))
}

fn can_scan_target_details(target: &crate::model::WorktreeInfo) -> bool {
    !target.bare && !target.prunable && target.path.exists()
}

fn read_working_tree_with_ignored(git: &Git, path: &Path) -> Result<WorktreeRemoveWorkingTree> {
    let output = git.run_in(
        path,
        [
            "status",
            "--porcelain=v1",
            "--ignored",
            "--untracked-files=all",
        ],
    )?;
    Ok(classify_working_tree_with_ignored(&output.stdout))
}

fn classify_working_tree_with_ignored(output: &str) -> WorktreeRemoveWorkingTree {
    let mut staged = 0;
    let mut unstaged = 0;
    let mut untracked = 0;
    let mut ignored = 0;
    let mut conflicts = Vec::new();

    for line in output.lines() {
        if line.len() < 4 {
            continue;
        }
        let code = &line[..2];
        let path = line[3..].to_string();
        let bytes = code.as_bytes();
        let (x, y) = (bytes[0] as char, bytes[1] as char);

        if code == "!!" {
            ignored += 1;
        } else if code == "??" {
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
    let clean =
        staged == 0 && unstaged == 0 && untracked == 0 && ignored == 0 && conflict_count == 0;

    WorktreeRemoveWorkingTree {
        clean,
        staged,
        unstaged,
        untracked,
        ignored,
        conflict_count,
        conflicts,
    }
}

fn is_conflict(x: char, y: char) -> bool {
    x == 'U' || y == 'U' || (x == 'D' && y == 'D') || (x == 'A' && y == 'A')
}

fn is_change(code: char) -> bool {
    code != ' ' && code != '?'
}

fn has_submodules(git: &Git, path: &Path) -> Result<bool> {
    if path.join(".gitmodules").exists() {
        return Ok(true);
    }
    let output = git.try_run_in(path, ["submodule", "status", "--recursive"])?;
    Ok(output.success && !output.stdout.trim().is_empty())
}

fn target_kind(target_index: usize, target: &crate::model::WorktreeInfo) -> &'static str {
    if target.bare {
        "bare"
    } else if target_index == 0 {
        "main"
    } else {
        "linked"
    }
}

fn collect_blocks(
    target: &WorktreeRemoveTarget,
    repository_git_common_dir: &Path,
) -> Vec<WorktreeRemoveBlock> {
    let mut blocks = Vec::new();
    if target
        .git_common_dir
        .as_ref()
        .is_some_and(|target_common_dir| target_common_dir != repository_git_common_dir)
    {
        push_block(&mut blocks, "target_family_mismatch");
    }
    if target.kind == "main" {
        push_block(&mut blocks, "target_is_main_worktree");
    }
    if target.kind == "bare" {
        push_block(&mut blocks, "target_is_bare_primary");
    }
    if target.kind != "linked" {
        push_block(&mut blocks, "target_not_linked_worktree");
    }
    if target.is_current_worktree {
        push_block(&mut blocks, "target_is_current_worktree");
    }
    if target.detached {
        push_block(&mut blocks, "target_detached");
    }
    if target.locked {
        push_block(&mut blocks, "target_locked");
    }
    if target.prunable {
        push_block(&mut blocks, "target_prunable");
    }
    if target.operation != Operation::None {
        push_block(&mut blocks, "operation_in_progress");
    }
    if target.working_tree.conflict_count > 0 {
        push_block(&mut blocks, "target_has_conflicts");
    }
    if target.working_tree.staged > 0 {
        push_block(&mut blocks, "target_has_staged_changes");
    }
    if target.working_tree.unstaged > 0 {
        push_block(&mut blocks, "target_has_unstaged_changes");
    }
    if target.working_tree.untracked > 0 {
        push_block(&mut blocks, "target_has_untracked_files");
    }
    if target.working_tree.ignored > 0 {
        push_block(&mut blocks, "target_has_ignored_files");
    }
    if target.has_submodules {
        push_block(&mut blocks, "target_has_submodules");
    }
    blocks
}

fn push_block(blocks: &mut Vec<WorktreeRemoveBlock>, code: &str) {
    blocks.push(WorktreeRemoveBlock {
        code: code.to_string(),
        severity: "hard_block".to_string(),
    });
}

impl WorktreeRemoveWorkingTree {
    fn clean() -> Self {
        Self {
            clean: true,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            ignored: 0,
            conflict_count: 0,
            conflicts: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::{Command, Output};

    use crate::git::worktree_remove::scan_worktree_remove_target;

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

    fn init_repo_with_commit(repo: &Path) {
        std::fs::create_dir_all(repo).expect("create repo");
        git(repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("README.md"), "hello\n").expect("write file");
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "-m", "initial"]);
    }

    fn block_codes(scan: &super::WorktreeRemoveScan) -> Vec<&str> {
        scan.blocks
            .iter()
            .map(|block| block.code.as_str())
            .collect()
    }

    #[test]
    fn scan_clean_linked_worktree_reports_preview_only_without_blocks() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let repo = temp_dir.path().join("repo");
        init_repo_with_commit(&repo);
        git(&repo, &["branch", "feature/remove-me"]);
        let target = temp_dir.path().join("repo.worktrees/remove-me");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                target.to_str().expect("target utf8"),
                "feature/remove-me",
            ],
        );
        let target = target.canonicalize().expect("canonical target");
        let before = run_git(&repo, &["worktree", "list", "--porcelain"]).stdout;

        let scan = scan_worktree_remove_target(&repo, &target).expect("scan target");

        assert_eq!(scan.execution_status, "preview_only");
        assert!(
            scan.blocks.is_empty(),
            "unexpected blocks: {:?}",
            scan.blocks
        );
        assert_eq!(scan.target.kind, "linked");
        assert_eq!(scan.target.worktree_list_path, target);
        assert_eq!(scan.target.branch.as_deref(), Some("feature/remove-me"));
        assert!(!scan.target.is_current_worktree);
        assert!(!scan.target.detached);
        assert!(!scan.target.locked);
        assert!(!scan.target.prunable);
        assert_eq!(scan.target.working_tree.staged, 0);
        assert_eq!(scan.target.working_tree.unstaged, 0);
        assert_eq!(scan.target.working_tree.untracked, 0);
        assert_eq!(scan.target.working_tree.ignored, 0);
        assert_eq!(scan.target.operation, crate::model::Operation::None);
        assert!(!scan.target.has_submodules);
        assert_eq!(
            run_git(&repo, &["worktree", "list", "--porcelain"]).stdout,
            before,
            "scan must not mutate worktree metadata"
        );
    }

    #[test]
    fn scan_uses_target_family_when_current_path_is_another_repo() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let repo = temp_dir.path().join("repo");
        let other = temp_dir.path().join("other");
        init_repo_with_commit(&repo);
        init_repo_with_commit(&other);
        git(&repo, &["branch", "feature/remove-me"]);
        let target = temp_dir.path().join("repo.worktrees/remove-me");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                target.to_str().expect("target utf8"),
                "feature/remove-me",
            ],
        );
        let target = target.canonicalize().expect("canonical target");

        let scan = scan_worktree_remove_target(&other, &target).expect("scan target");

        assert_eq!(scan.execution_status, "preview_only");
        assert_eq!(
            scan.repository.git_common_dir,
            repo.join(".git").canonicalize().expect("canonical git dir")
        );
        assert_eq!(scan.target.worktree_list_path, target);
        assert!(scan.blocks.is_empty());
    }

    #[test]
    fn scan_requires_absolute_target_path() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let repo = temp_dir.path().join("repo");
        init_repo_with_commit(&repo);

        let error = scan_worktree_remove_target(&repo, Path::new("relative-target"))
            .expect_err("relative path should fail");

        assert!(error.to_string().contains("target_path_not_absolute"));
    }

    #[test]
    fn scan_rejects_absolute_path_that_is_not_exact_worktree_list_entry() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let repo = temp_dir.path().join("repo");
        init_repo_with_commit(&repo);
        let absent = temp_dir.path().join("repo.worktrees/absent");

        let error = scan_worktree_remove_target(&repo, &absent)
            .expect_err("absent absolute path should fail");

        assert!(error
            .to_string()
            .contains("target_path_not_exact_worktree_list_entry"));
    }

    #[test]
    fn scan_blocks_main_and_current_worktree() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let repo = temp_dir.path().join("repo");
        init_repo_with_commit(&repo);
        let repo = repo.canonicalize().expect("canonical repo");

        let scan = scan_worktree_remove_target(&repo, &repo).expect("scan main");
        let codes = block_codes(&scan);

        assert!(codes.contains(&"target_is_main_worktree"));
        assert!(codes.contains(&"target_is_current_worktree"));
        assert_eq!(scan.execution_status, "blocked");
    }

    #[test]
    fn scan_blocks_staged_unstaged_untracked_and_ignored_files() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let repo = temp_dir.path().join("repo");
        init_repo_with_commit(&repo);
        std::fs::write(repo.join(".gitignore"), "*.log\n").expect("write gitignore");
        git(&repo, &["add", ".gitignore"]);
        git(&repo, &["commit", "-q", "-m", "ignore logs"]);
        git(&repo, &["branch", "feature/dirty"]);
        let target = temp_dir.path().join("repo.worktrees/dirty");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                target.to_str().expect("target utf8"),
                "feature/dirty",
            ],
        );
        let target = target.canonicalize().expect("canonical target");
        std::fs::write(target.join("README.md"), "changed\n").expect("write unstaged");
        std::fs::write(target.join("staged.txt"), "staged\n").expect("write staged");
        git(&target, &["add", "staged.txt"]);
        std::fs::write(target.join("scratch.txt"), "local\n").expect("write untracked");
        std::fs::write(target.join("debug.log"), "ignored\n").expect("write ignored");

        let scan = scan_worktree_remove_target(&repo, &target).expect("scan dirty");
        let codes = block_codes(&scan);

        assert!(codes.contains(&"target_has_staged_changes"));
        assert!(codes.contains(&"target_has_unstaged_changes"));
        assert!(codes.contains(&"target_has_untracked_files"));
        assert!(codes.contains(&"target_has_ignored_files"));
        assert_eq!(scan.target.working_tree.staged, 1);
        assert_eq!(scan.target.working_tree.unstaged, 1);
        assert_eq!(scan.target.working_tree.untracked, 1);
        assert_eq!(scan.target.working_tree.ignored, 1);
        assert_eq!(scan.execution_status, "blocked");
    }

    #[test]
    fn scan_blocks_detached_and_locked_worktree() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let repo = temp_dir.path().join("repo");
        init_repo_with_commit(&repo);
        let target = temp_dir.path().join("repo.worktrees/detached");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                target.to_str().expect("target utf8"),
                "HEAD",
            ],
        );
        let target = target.canonicalize().expect("canonical target");
        git(
            &repo,
            &["worktree", "lock", target.to_str().expect("target utf8")],
        );

        let scan = scan_worktree_remove_target(&repo, &target).expect("scan target");
        let codes = block_codes(&scan);

        assert!(codes.contains(&"target_detached"));
        assert!(codes.contains(&"target_locked"));
        assert_eq!(scan.execution_status, "blocked");
    }

    #[test]
    fn scan_blocks_real_merge_conflict_and_in_progress_operation() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let repo = temp_dir.path().join("repo");
        init_repo_with_commit(&repo);
        std::fs::write(repo.join("conflict.txt"), "base\n").expect("write base");
        git(&repo, &["add", "conflict.txt"]);
        git(&repo, &["commit", "-q", "-m", "base conflict file"]);
        git(&repo, &["branch", "feature/conflict"]);
        std::fs::write(repo.join("conflict.txt"), "main\n").expect("write main");
        git(&repo, &["commit", "-q", "-am", "main side"]);
        let target = temp_dir.path().join("repo.worktrees/conflict");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                target.to_str().expect("target utf8"),
                "feature/conflict",
            ],
        );
        let target = target.canonicalize().expect("canonical target");
        std::fs::write(target.join("conflict.txt"), "feature\n").expect("write feature");
        git(&target, &["commit", "-q", "-am", "feature side"]);
        let merge = run_git(&target, &["merge", "main"]);
        assert!(
            !merge.status.success(),
            "merge should conflict: {}",
            String::from_utf8_lossy(&merge.stderr)
        );

        let scan = scan_worktree_remove_target(&repo, &target).expect("scan target");
        let codes = block_codes(&scan);

        assert_eq!(scan.target.operation, crate::model::Operation::Merging);
        assert_eq!(scan.target.working_tree.conflict_count, 1);
        assert!(codes.contains(&"operation_in_progress"));
        assert!(codes.contains(&"target_has_conflicts"));
        assert_eq!(scan.execution_status, "blocked");
    }

    #[test]
    fn scan_blocks_real_submodule_config() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let repo = temp_dir.path().join("repo");
        init_repo_with_commit(&repo);
        git(&repo, &["branch", "feature/submodule"]);
        let target = temp_dir.path().join("repo.worktrees/submodule");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                target.to_str().expect("target utf8"),
                "feature/submodule",
            ],
        );
        let target = target.canonicalize().expect("canonical target");
        std::fs::write(
            target.join(".gitmodules"),
            "[submodule \"dep\"]\n\tpath = dep\n\turl = https://example.com/dep.git\n",
        )
        .expect("write gitmodules");
        git(&target, &["add", ".gitmodules"]);
        git(&target, &["commit", "-q", "-m", "add submodule config"]);

        let scan = scan_worktree_remove_target(&repo, &target).expect("scan target");
        let codes = block_codes(&scan);

        assert!(scan.target.has_submodules);
        assert!(codes.contains(&"target_has_submodules"));
        assert_eq!(scan.execution_status, "blocked");
    }

    #[test]
    fn scan_blocks_prunable_entry_from_current_family_when_target_is_missing() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let repo = temp_dir.path().join("repo");
        init_repo_with_commit(&repo);
        git(&repo, &["branch", "feature/prunable"]);
        let target = temp_dir.path().join("repo.worktrees/prunable");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                target.to_str().expect("target utf8"),
                "feature/prunable",
            ],
        );
        let target = target.canonicalize().expect("canonical target");
        std::fs::remove_dir_all(&target).expect("remove worktree directory");

        let scan = scan_worktree_remove_target(&repo, &target).expect("scan target");
        let codes = block_codes(&scan);

        assert!(scan.target.prunable);
        assert!(codes.contains(&"target_prunable"));
        assert_eq!(scan.execution_status, "blocked");
    }

    #[test]
    fn block_collector_covers_operation_conflicts_prunable_and_submodules() {
        let target = super::WorktreeRemoveTarget {
            input_path: Path::new("/repo.worktrees/feature").to_path_buf(),
            canonical_path: Path::new("/repo.worktrees/feature").to_path_buf(),
            worktree_list_path: Path::new("/repo.worktrees/feature").to_path_buf(),
            list_index: 1,
            kind: "linked".to_string(),
            head: Some("abc123".to_string()),
            branch: Some("feature".to_string()),
            detached: false,
            locked: false,
            prunable: true,
            is_current_worktree: false,
            worktree_git_dir: Some(Path::new("/repo/.git/worktrees/feature").to_path_buf()),
            git_common_dir: Some(Path::new("/repo/.git").to_path_buf()),
            operation: crate::model::Operation::Rebasing,
            working_tree: super::WorktreeRemoveWorkingTree {
                clean: false,
                staged: 0,
                unstaged: 0,
                untracked: 0,
                ignored: 0,
                conflict_count: 1,
                conflicts: vec!["both.txt".to_string()],
            },
            has_submodules: true,
        };

        let blocks = super::collect_blocks(&target, Path::new("/repo/.git"));
        let codes: Vec<&str> = blocks.iter().map(|block| block.code.as_str()).collect();

        assert!(codes.contains(&"target_prunable"));
        assert!(codes.contains(&"operation_in_progress"));
        assert!(codes.contains(&"target_has_conflicts"));
        assert!(codes.contains(&"target_has_submodules"));
    }
}
