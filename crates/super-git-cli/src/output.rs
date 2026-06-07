use std::path::Path;

use anyhow::Result;
use serde::Serialize;
use serde_json::json;
use super_git_core::config::store::{
    AppConfig, AppHome, ConfigUpdateResult, ForgetRepositoryResult, SavedRepository,
};
use super_git_core::config::template::ConfigValidationReport;
use super_git_core::model::{
    ExecuteResult, Operation, PreviewPlan, RepoState, RiskLevel, StatusOutput, UndoResult,
    WorktreeCreatePlan, WorktreeInfo, WorktreeKind, WorktreeRemovePlan, INSPECT_SCHEMA_VERSION,
};
use super_git_core::SuperGitError;

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
            let mut value = json!({
                "ok": false,
                "error": {
                    "message": err.to_string(),
                    "causes": causes,
                }
            });
            if let Some(details) = structured_error_details(err) {
                value["error"]["details"] = details;
            }

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

fn structured_error_details(err: &anyhow::Error) -> Option<serde_json::Value> {
    err.chain().find_map(|cause| {
        let error = cause.downcast_ref::<SuperGitError>()?;
        match error {
            SuperGitError::ExecutePartialFailure {
                action,
                message,
                execution_record_path,
                target_path,
                target_path_exists,
                worktree_list_entry_present,
            } => Some(json!({
                "status": "failed_partial",
                "action": action,
                "message": message,
                "execution_record_path": execution_record_path,
                "observed": {
                    "target_path": target_path,
                    "target_path_exists": target_path_exists,
                    "worktree_list_entry_present": worktree_list_entry_present,
                },
                "cleanup": {
                    "automatic_undo_available": false,
                    "safe_next": "inspect_cleanup_record",
                    "reason": "Git may have created partial worktree state; re-inspect the execution record before cleanup."
                }
            })),
            _ => None,
        }
    })
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

pub fn print_config_path(mode: OutputMode, app_home: &AppHome) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(app_home),
        OutputMode::Human => {
            println!("super-git config path");
            println!("Home: {}", app_home.home.display());
            println!("Source: {}", app_home.source.as_str());
            println!("Config: {}", app_home.config_file.display());
            Ok(())
        }
    }
}

pub fn print_config_show(mode: OutputMode, app_home: &AppHome, config: &AppConfig) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(json!({
            "location": app_home,
            "config": config,
        })),
        OutputMode::Human => {
            println!("super-git config");
            println!("Home: {}", app_home.home.display());
            println!("Source: {}", app_home.source.as_str());
            println!("Config: {}", app_home.config_file.display());
            println!("Repositories: {}", config.repositories.len());
            for (index, repo) in config.repositories.iter().enumerate() {
                println!(
                    "  {:<4} {} ({})",
                    index + 1,
                    repo.name,
                    repo.git_common_dir.display()
                );
            }
            Ok(())
        }
    }
}

pub fn print_config_validate(
    mode: OutputMode,
    app_home: &AppHome,
    report: &ConfigValidationReport,
) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(json!({
            "location": app_home,
            "valid": report.valid,
            "issues": report.issues,
        })),
        OutputMode::Human => {
            println!("super-git config validate");
            println!("Config: {}", app_home.config_file.display());
            if report.valid {
                println!("Status: valid");
            } else {
                println!("Status: invalid");
                for issue in &report.issues {
                    println!("  {}: {} ({})", issue.field, issue.message, issue.code);
                }
            }
            Ok(())
        }
    }
}

pub fn print_config_update(
    mode: OutputMode,
    app_home: &AppHome,
    result: &ConfigUpdateResult,
) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(json!({
            "location": app_home,
            "config": result.config,
            "changed": result.changed,
            "validation": result.validation,
        })),
        OutputMode::Human => {
            println!("super-git config set-worktree-template");
            println!("Config: {}", app_home.config_file.display());
            println!("Changed: {}", result.changed);
            println!(
                "Parent template: {}",
                result.config.settings.worktree.parent_template
            );
            println!(
                "Name template: {}",
                result.config.settings.worktree.name_template
            );
            println!(
                "Ref slug algorithm: {}",
                result.config.settings.worktree.ref_slug_algorithm
            );
            Ok(())
        }
    }
}

pub fn print_repo_save(mode: OutputMode, repository: &SavedRepository, added: bool) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(json!({
            "repository": repository,
            "added": added,
        })),
        OutputMode::Human => {
            if added {
                println!(
                    "Saved repository family: {} ({})",
                    repository.name, repository.id
                );
            } else {
                println!(
                    "Already saved repository family: {} ({})",
                    repository.name, repository.id
                );
            }
            Ok(())
        }
    }
}

pub fn print_repo_add(
    mode: OutputMode,
    repository: &SavedRepository,
    requested_path: &Path,
    added: bool,
) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(json!({
            "path": requested_path,
            "repository": repository,
            "added": added,
        })),
        OutputMode::Human => print_repo_save(mode, repository, added),
    }
}

pub fn print_repo_forget(mode: OutputMode, result: &ForgetRepositoryResult) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(json!({
            "target": result.target,
            "repository": result.repository,
            "removed": result.removed,
            "matched_by": result.matched_by,
            "remaining_repositories": result.remaining_repositories,
            "registry_only": result.registry_only,
            "filesystem_deleted": result.filesystem_deleted,
        })),
        OutputMode::Human => {
            println!(
                "Forgot repository family: {} ({})",
                result.repository.name, result.repository.id
            );
            println!("Matched by: {}", result.matched_by.as_str());
            println!("Remaining repositories: {}", result.remaining_repositories);
            println!("Registry only: yes");
            println!("Filesystem deleted: no");
            Ok(())
        }
    }
}

pub fn print_repo_list(mode: OutputMode, repositories: &[SavedRepository]) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(json!({ "repositories": repositories })),
        OutputMode::Human => {
            if repositories.is_empty() {
                println!("No repositories registered yet.");
                return Ok(());
            }

            println!("{:<4} {:<24} {:<22} Git common dir", "#", "Name", "Kind");
            for (index, repo) in repositories.iter().enumerate() {
                println!(
                    "{:<4} {:<24} {:<22} {}",
                    index + 1,
                    repo.name,
                    repo.kind.as_str(),
                    repo.git_common_dir.display()
                );
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
            "schema_version": INSPECT_SCHEMA_VERSION,
            "repository": state.root,
            "worktree_context": state.worktree_context,
            "head": state.head,
            "upstream": state.upstream,
            "working_tree": state.working_tree,
            "operation": state.operation,
            "next": state.next,
            "warnings": state.warnings,
            "summary": state.summary,
            "risk_hint": state.risk_hint,
        })),
        OutputMode::Human => {
            println!("Repository: {}", state.root.display());
            println!(
                "Summary: {} ({})",
                state.summary.state, state.summary.message
            );
            println!("Risk hint: {}", risk_level_label(state.risk_hint.level));

            let wc = &state.worktree_context;
            println!(
                "Worktree: {} (family {}, linked {})",
                worktree_kind_label(wc.kind),
                wc.family_count,
                wc.linked_count
            );
            if let Some(main) = &wc.main {
                if wc.kind == WorktreeKind::Linked {
                    println!("  main: {}", main.display());
                }
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

            if !state.warnings.is_empty() {
                println!("Warnings:");
                for warning in &state.warnings {
                    println!("  - {}: {}", warning.code, warning.message);
                }
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

            if !state.next.allowed.is_empty() {
                println!("Preview candidates:");
                for next in &state.next.allowed {
                    println!("  - {} ({})", next.kind, next.reason);
                }
            }

            if !state.next.blocked.is_empty() {
                println!("Blocked next actions:");
                for next in &state.next.blocked {
                    println!("  - {} ({})", next.kind, next.reason);
                }
            }

            if !state.next.needs_human_review.is_empty() {
                println!("Needs human review before execute:");
                for next in &state.next.needs_human_review {
                    println!("  - {} ({})", next.kind, next.reason);
                }
            }
            Ok(())
        }
    }
}

pub fn print_preview_plan(mode: OutputMode, plan: &PreviewPlan) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(plan),
        OutputMode::Human => {
            println!("Preview: {}", plan.action.kind);
            println!("Plan: {}", plan.plan_id);
            println!("Repository: {}", plan.repository.display());
            println!("Scope: {}", plan.action.scope);
            println!("Paths: {}", plan.action.resolved_paths.len());
            for path in &plan.action.resolved_paths {
                println!("  - {path}");
            }
            println!("Risk: {} / {}", plan.risk.severity, plan.risk.reversibility);
            println!("Writes now: no");
            Ok(())
        }
    }
}

pub fn print_worktree_create_plan(mode: OutputMode, plan: &WorktreeCreatePlan) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(plan),
        OutputMode::Human => {
            println!("Preview: {}", plan.action.kind);
            println!("Plan: {}", plan.plan_id);
            println!("Repository: {}", plan.repository.selected_from.display());
            println!("Ref: {}", plan.action.options.ref_name);
            println!("Target: {}", plan.target.path.display());
            println!("Execution: {}", plan.execution.status);
            if !plan.execution.blocked_reasons.is_empty() {
                println!("Blocked reasons:");
                for reason in &plan.execution.blocked_reasons {
                    println!("  - {} ({})", reason.code, reason.severity);
                }
            }
            println!("Risk: {} / {}", plan.risk.severity, plan.risk.reversibility);
            println!("Writes now: no");
            Ok(())
        }
    }
}

pub fn print_worktree_remove_plan(mode: OutputMode, plan: &WorktreeRemovePlan) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(plan),
        OutputMode::Human => {
            println!("Preview: {}", plan.action.kind);
            println!("Plan: {}", plan.plan_id);
            println!("Repository: {}", plan.repository.selected_from.display());
            println!("Target: {}", plan.target.worktree_list_path.display());
            println!("Execution: {}", plan.execution.status);
            if !plan.execution.blocked_reasons.is_empty() {
                println!("Blocked reasons:");
                for reason in &plan.execution.blocked_reasons {
                    println!("  - {} ({})", reason.code, reason.severity);
                }
            }
            println!("Execute supported: no");
            println!("Risk: {} / {}", plan.risk.severity, plan.risk.reversibility);
            println!("Undo: {}", plan.undo_strategy.kind);
            println!("Writes now: no");
            Ok(())
        }
    }
}

pub fn print_execute_result(mode: OutputMode, result: &ExecuteResult) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(result),
        OutputMode::Human => {
            println!("Executed: {}", result.action);
            println!("Plan: {}", result.plan_id);
            println!("Repository: {}", result.repository.display());
            println!("Undo token: {}", result.undo_token.kind());
            Ok(())
        }
    }
}

pub fn print_undo_result(mode: OutputMode, result: &UndoResult) -> Result<()> {
    match mode {
        OutputMode::Json => emit_success(result),
        OutputMode::Human => {
            println!("Undone: {}", result.action);
            println!("Plan: {}", result.plan_id);
            println!("Repository: {}", result.repository.display());
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

fn risk_level_label(level: RiskLevel) -> &'static str {
    match level {
        RiskLevel::Low => "low",
        RiskLevel::Medium => "medium",
        RiskLevel::High => "high",
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
