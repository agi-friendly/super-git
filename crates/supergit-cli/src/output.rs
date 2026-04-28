use std::path::Path;

use supergit_core::model::{Repository, StatusOutput, WorktreeInfo};

pub fn print_doctor(git_version: &str, config_path: &Path) {
    println!("super-git doctor");
    println!("Git: OK ({git_version})");
    println!("OS: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    println!("Config: {}", config_path.display());
}

pub fn print_repo_add(path: &Path, added: bool) {
    if added {
        println!("Added repository: {}", path.display());
    } else {
        println!("Already registered: {}", path.display());
    }
}

pub fn print_repo_list(repositories: &[Repository]) {
    if repositories.is_empty() {
        println!("No repositories registered yet.");
        return;
    }

    println!("{:<4} Path", "#");

    for (index, repo) in repositories.iter().enumerate() {
        println!("{:<4} {}", index + 1, repo.path.display());
    }
}

pub fn print_status(path: &Path, status: &StatusOutput) {
    println!("Repository: {}", path.display());

    if let Some(branch) = &status.branch_header {
        println!("Branch: {branch}");
    }

    if status.is_clean() {
        println!("Status: clean");
        return;
    }

    println!("Status:");
    for entry in &status.entries {
        println!("  {entry}");
    }
}

pub fn print_worktrees(worktrees: &[WorktreeInfo]) {
    if worktrees.is_empty() {
        println!("No worktrees found.");
        return;
    }

    println!("{:<12} {:<18} {:<10} Path", "HEAD", "Branch", "State");

    for worktree in worktrees {
        let head = worktree.head.as_deref().unwrap_or("-");
        let short_head = if head.len() > 12 { &head[..12] } else { head };
        let branch = worktree.branch.as_deref().unwrap_or("-");
        let state = if worktree.bare {
            "bare"
        } else if worktree.detached {
            "detached"
        } else {
            "normal"
        };

        println!(
            "{:<12} {:<18} {:<10} {}",
            short_head,
            branch,
            state,
            worktree.path.display()
        );
    }
}
