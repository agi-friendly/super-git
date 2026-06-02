use std::path::Path;

use anyhow::Result;
use serde::Serialize;
use serde_json::json;
use super_git_core::model::{Repository, StatusOutput, WorktreeInfo};

/// 출력 표현 방식. 기본은 AI/기계 친화적인 JSON이고,
/// 사람이 직접 읽을 때만 `--human`으로 Human을 고른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Json,
    Human,
}

/// 단일 JSON 출력 지점. C3에서 core의 report 레이어로 옮겨갈 자리다.
fn emit_json(value: impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

pub fn print_doctor(mode: OutputMode, git_version: &str, config_path: &Path) -> Result<()> {
    match mode {
        OutputMode::Json => emit_json(json!({
            "git_version": git_version,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "config_path": config_path,
        })),
        OutputMode::Human => {
            println!("super-git doctor");
            println!("Git: OK ({git_version})");
            println!("OS: {} {}", std::env::consts::OS, std::env::consts::ARCH);
            println!("Config: {}", config_path.display());
            Ok(())
        }
    }
}

pub fn print_repo_add(mode: OutputMode, path: &Path, added: bool) -> Result<()> {
    match mode {
        OutputMode::Json => emit_json(json!({
            "path": path,
            "added": added,
        })),
        OutputMode::Human => {
            if added {
                println!("Added repository: {}", path.display());
            } else {
                println!("Already registered: {}", path.display());
            }
            Ok(())
        }
    }
}

pub fn print_repo_list(mode: OutputMode, repositories: &[Repository]) -> Result<()> {
    match mode {
        OutputMode::Json => emit_json(json!({ "repositories": repositories })),
        OutputMode::Human => {
            if repositories.is_empty() {
                println!("No repositories registered yet.");
                return Ok(());
            }

            println!("{:<4} Path", "#");
            for (index, repo) in repositories.iter().enumerate() {
                println!("{:<4} {}", index + 1, repo.path.display());
            }
            Ok(())
        }
    }
}

pub fn print_status(mode: OutputMode, path: &Path, status: &StatusOutput) -> Result<()> {
    match mode {
        OutputMode::Json => emit_json(json!({
            "repository": path,
            "branch_header": status.branch_header,
            "entries": status.entries,
            "clean": status.is_clean(),
        })),
        OutputMode::Human => {
            println!("Repository: {}", path.display());

            if let Some(branch) = &status.branch_header {
                println!("Branch: {branch}");
            }

            if status.is_clean() {
                println!("Status: clean");
                return Ok(());
            }

            println!("Status:");
            for entry in &status.entries {
                println!("  {entry}");
            }
            Ok(())
        }
    }
}

pub fn print_worktrees(mode: OutputMode, worktrees: &[WorktreeInfo]) -> Result<()> {
    match mode {
        OutputMode::Json => emit_json(json!({ "worktrees": worktrees })),
        OutputMode::Human => {
            if worktrees.is_empty() {
                println!("No worktrees found.");
                return Ok(());
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
            Ok(())
        }
    }
}
