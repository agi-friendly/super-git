use std::path::Path;

use anyhow::Result;
use serde::Serialize;
use serde_json::json;
use super_git_core::model::{
    Operation, RepoState, Repository, StatusOutput, WorktreeInfo, WorktreeKind,
};

/// 출력 표현 방식. 기본은 AI/기계 친화적인 JSON이고,
/// 사람이 직접 읽을 때만 `--human`으로 Human을 고른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Json,
    Human,
}

/// 성공 출력의 단일 JSON 지점. 실패의 `{ ok: false, error }`와 대칭이 되도록
/// 성공도 `{ ok: true, data: {...} }` envelope로 감싼다. AI가 `ok` 필드 하나로 분기한다.
/// C3에서 core의 report 레이어로 옮겨갈 자리다.
fn emit_success(data: impl Serialize) -> Result<()> {
    let envelope = json!({ "ok": true, "data": data });
    println!("{}", serde_json::to_string_pretty(&envelope)?);
    Ok(())
}

/// 실패도 출력 계약을 지킨다.
/// JSON 모드: stdout에 `{ "ok": false, "error": {...} }`를 내보낸다.
/// 성공/실패를 같은 스트림(stdout)에서 파싱하고 exit code로 구분하도록 한다.
/// Human 모드: 기존처럼 stderr에 사람용 텍스트를 쓴다.
pub fn print_error(mode: OutputMode, err: &anyhow::Error) {
    match mode {
        OutputMode::Json => {
            // anyhow의 context 체인을 펼쳐 최상위 메시지와 원인들을 분리한다.
            let causes: Vec<String> = err.chain().skip(1).map(|cause| cause.to_string()).collect();
            let value = json!({
                "ok": false,
                "error": {
                    "message": err.to_string(),
                    "causes": causes,
                }
            });

            // 에러 직렬화 자체가 실패하는 극단적 경우의 최후 수단.
            match serde_json::to_string_pretty(&value) {
                Ok(json) => println!("{json}"),
                Err(_) => eprintln!("Error: {err:#}"),
            }
        }
        OutputMode::Human => {
            eprintln!("Error: {err:#}");
        }
    }
}

/// clap 파싱 에러도 같은 출력 계약을 따른다.
/// JSON 모드: stdout에 envelope. Human 모드: clap 기본 렌더링(usage 포함)을 stderr에.
pub fn print_parse_error(mode: OutputMode, err: &clap::Error) {
    match mode {
        OutputMode::Json => {
            let value = json!({
                "ok": false,
                "error": {
                    "message": "invalid command-line arguments",
                    "causes": [err.to_string()],
                }
            });

            match serde_json::to_string_pretty(&value) {
                Ok(json) => println!("{json}"),
                Err(_) => eprintln!("{err}"),
            }
        }
        OutputMode::Human => {
            let _ = err.print();
        }
    }
}

pub fn print_doctor(mode: OutputMode, git_version: &str, config_path: &Path) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(json!({
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
        OutputMode::Json => emit_success(json!({
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
        OutputMode::Json => emit_success(json!({ "repositories": repositories })),
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
        OutputMode::Json => emit_success(json!({
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

pub fn print_inspect(mode: OutputMode, state: &RepoState) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(json!({
            "repository": state.root,
            "worktree_context": state.worktree_context,
            "head": state.head,
            "upstream": state.upstream,
            "working_tree": state.working_tree,
            "operation": state.operation,
            "allowed_next": state.allowed_next,
        })),
        OutputMode::Human => {
            println!("Repository: {}", state.root.display());

            let wc = &state.worktree_context;
            println!(
                "Worktree: {} (family {}, linked {})",
                worktree_kind_label(wc.kind),
                wc.family_count,
                wc.linked_count
            );
            if wc.kind == WorktreeKind::Linked {
                println!("  main: {}", wc.main.display());
            }

            match &state.head.branch {
                Some(branch) => println!("Branch: {branch}"),
                None => println!("Branch: (detached)"),
            }
            match &state.head.commit {
                Some(commit) => println!("HEAD: {commit}"),
                None => println!("HEAD: (unborn)"),
            }
            match &state.upstream {
                Some(upstream) => println!(
                    "Upstream: {} (ahead {}, behind {})",
                    upstream.name, upstream.ahead, upstream.behind
                ),
                None => println!("Upstream: (none)"),
            }

            let wt = &state.working_tree;
            if wt.clean {
                println!("Working tree: clean");
            } else {
                println!(
                    "Working tree: staged {}, unstaged {}, untracked {}, conflicts {}",
                    wt.staged, wt.unstaged, wt.untracked, wt.conflict_count
                );
                for conflict in &wt.conflicts {
                    println!("  conflict: {conflict}");
                }
            }

            println!("Operation: {}", operation_label(state.operation));

            if !state.allowed_next.is_empty() {
                println!("Next:");
                for next in &state.allowed_next {
                    println!("  - {} ({})", next.kind, next.reason);
                }
            }
            Ok(())
        }
    }
}

fn worktree_kind_label(kind: WorktreeKind) -> &'static str {
    match kind {
        WorktreeKind::Main => "main",
        WorktreeKind::Linked => "linked",
        WorktreeKind::Bare => "bare",
        WorktreeKind::Unknown => "unknown",
    }
}

fn operation_label(operation: Operation) -> &'static str {
    match operation {
        Operation::None => "none",
        Operation::Merging => "merging",
        Operation::Rebasing => "rebasing",
        Operation::Applying => "applying",
        Operation::CherryPicking => "cherry-picking",
        Operation::Reverting => "reverting",
        Operation::Bisecting => "bisecting",
    }
}

pub fn print_worktrees(mode: OutputMode, worktrees: &[WorktreeInfo]) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(json!({ "worktrees": worktrees })),
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
