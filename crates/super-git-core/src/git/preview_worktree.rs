use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::config::store::{
    resolve_repository_in_config, AppConfig, AppHome, ConfigStore, SavedRepository,
    WorktreeSettings,
};
use crate::config::template::validate_config;
use crate::git::command::Git;
use crate::git::{worktree, worktree_plan};
use crate::model::{
    ActionRisk, BranchOccupancy, WorktreeBlockedReason, WorktreeCreateAction,
    WorktreeCreateConfigUsed, WorktreeCreateExecution, WorktreeCreateOptions, WorktreeCreatePlan,
    WorktreeCreatePrecondition, WorktreeCreateRepository, WorktreeCreateTarget,
    WorktreeCreateUndoPreview, WorktreeCreateUndoStrategy, WorktreeFamilySnapshot, WorktreeInfo,
    WorktreeParentCreationView, WorktreeRefPolicy, WorktreeReferenceCommands,
    WorktreeSnapshotEntry, WorktreeSourceRef, WorktreeTemplateConfig,
    WorktreeTemplateVariablesView, WORKTREE_PLAN_SCHEMA_VERSION,
};
use crate::{Result, SuperGitError};

const ACTION_WORKTREE_CREATE: &str = "worktree_create";

pub fn preview_worktree_create(
    current_path: &Path,
    app_home: &AppHome,
    store: &ConfigStore,
    repo_selector: Option<String>,
    ref_name: String,
) -> Result<WorktreeCreatePlan> {
    let config = store.load()?;
    ensure_config_valid(&config)?;
    let repository = match repo_selector.as_deref() {
        Some(selector) => resolve_repository_selector(&config, selector)?,
        None => SavedRepository::from_path(current_path)?,
    };
    let selected_from = selected_from_path(&repository);
    let worktrees = worktree::list_worktrees(&selected_from)?;
    let family_snapshot = family_snapshot(&worktrees)?;
    let source_ref = classify_source_ref(&selected_from, &ref_name)?;
    let ref_policy = ref_policy(&source_ref);
    let target_ref_name = target_ref_name(&source_ref, &ref_name);
    let target = worktree_plan::resolve_worktree_target(
        &repository,
        &config.settings.worktree,
        &target_ref_name,
        &worktrees,
    )
    .map(target_from_resolved)?;

    let mut blocked_reasons = Vec::new();
    collect_source_blocks(&source_ref, &mut blocked_reasons);
    collect_branch_blocks(&source_ref, &family_snapshot, &mut blocked_reasons);
    collect_target_blocks(&target, &mut blocked_reasons);

    let execution_status = if blocked_reasons.is_empty() {
        "preview_only"
    } else {
        "blocked"
    };
    let execution = WorktreeCreateExecution {
        status: execution_status.to_string(),
        super_git_execute_required: true,
        raw_git_allowed: false,
        suggested_super_git_command: (execution_status == "executable").then(|| {
            vec![
                "super-git".to_string(),
                "execute".to_string(),
                "--plan".to_string(),
                "<plan-file>".to_string(),
            ]
        }),
        blocked_reasons,
    };

    let risk = ActionRisk {
        severity: "medium".to_string(),
        reversibility: "reversible_if_unchanged".to_string(),
        requires_human_confirmation: false,
    };
    let undo_strategy = WorktreeCreateUndoStrategy {
        kind: "remove_created_worktree_if_clean".to_string(),
        deletes_branch: false,
        deletes_history: false,
    };
    let undo_preview = WorktreeCreateUndoPreview {
        kind: "remove_created_worktree_if_clean".to_string(),
        available_after_execute: execution.status == "executable",
        limitations: vec![
            "Undo removes only the worktree created by super-git.".to_string(),
            "Undo refuses if the created worktree is dirty, locked, moved, or no longer matches the execute record.".to_string(),
            "Undo does not delete branch refs or commits.".to_string(),
        ],
    };

    let reference_ref = source_ref
        .full_ref
        .as_deref()
        .unwrap_or(&source_ref.input)
        .to_string();
    let mut plan = WorktreeCreatePlan {
        schema_version: WORKTREE_PLAN_SCHEMA_VERSION.to_string(),
        plan_id: String::new(),
        action: WorktreeCreateAction {
            kind: ACTION_WORKTREE_CREATE.to_string(),
            options: WorktreeCreateOptions {
                repo_selector,
                ref_name,
            },
        },
        repository: WorktreeCreateRepository {
            family_id: repository.id,
            kind: repository.kind.as_str().to_string(),
            git_common_dir: repository.git_common_dir,
            main_worktree: repository.main_worktree,
            selected_from,
        },
        config_used: config_used(app_home, &config.settings.worktree, &config)?,
        source_ref,
        ref_policy,
        target,
        family_snapshot,
        preconditions: preconditions(execution_status),
        execution,
        risk,
        effects: effects_for(execution_status),
        reference_commands: WorktreeReferenceCommands {
            semantics: "documentation_only".to_string(),
            never_execute_directly: true,
            commands: vec![vec![
                "git".to_string(),
                "worktree".to_string(),
                "add".to_string(),
                "<target-path>".to_string(),
                reference_ref,
            ]],
        },
        undo_strategy,
        undo_preview,
    };
    plan.plan_id = compute_worktree_plan_id(&plan)?;
    Ok(plan)
}

fn ensure_config_valid(config: &AppConfig) -> Result<()> {
    let report = validate_config(config);
    if let Some(issue) = report.issues.first() {
        return Err(SuperGitError::ConfigValidationFailed {
            field: issue.field.clone(),
            code: issue.code.clone(),
            message: issue.message.clone(),
        });
    }

    Ok(())
}

fn resolve_repository_selector(config: &AppConfig, selector: &str) -> Result<SavedRepository> {
    match resolve_repository_in_config(config, selector) {
        Ok(resolved) => Ok(resolved.repository),
        Err(SuperGitError::RepositoryNotFound { .. }) if looks_like_path(selector) => {
            SavedRepository::from_path(Path::new(selector))
        }
        Err(err) => Err(err),
    }
}

fn looks_like_path(selector: &str) -> bool {
    selector == "."
        || selector.contains('/')
        || selector.contains('\\')
        || Path::new(selector).is_absolute()
}

fn selected_from_path(repository: &SavedRepository) -> PathBuf {
    repository
        .main_worktree
        .clone()
        .unwrap_or_else(|| repository.saved_from.clone())
}

fn classify_source_ref(path: &Path, input: &str) -> Result<WorktreeSourceRef> {
    let git = Git::default();

    if let Some(commit) = verify_commitish(&git, path, &format!("refs/heads/{input}"))? {
        return Ok(WorktreeSourceRef {
            input: input.to_string(),
            kind: "local_branch".to_string(),
            full_ref: Some(format!("refs/heads/{input}")),
            resolved_commit: Some(commit),
            supported_for_execute: true,
        });
    }

    if let Some(commit) = verify_commitish(&git, path, &format!("refs/tags/{input}"))? {
        return Ok(WorktreeSourceRef {
            input: input.to_string(),
            kind: "tag".to_string(),
            full_ref: Some(format!("refs/tags/{input}")),
            resolved_commit: Some(commit),
            supported_for_execute: true,
        });
    }

    if let Some(commit) = verify_commitish(&git, path, &format!("refs/remotes/{input}"))? {
        return Ok(WorktreeSourceRef {
            input: input.to_string(),
            kind: "remote_tracking_branch".to_string(),
            full_ref: Some(format!("refs/remotes/{input}")),
            resolved_commit: Some(commit),
            supported_for_execute: false,
        });
    }

    if looks_like_commit(input) {
        if let Some(commit) = verify_commitish(&git, path, input)? {
            return Ok(WorktreeSourceRef {
                input: input.to_string(),
                kind: "commit".to_string(),
                full_ref: None,
                resolved_commit: Some(commit),
                supported_for_execute: true,
            });
        }
    }

    Ok(WorktreeSourceRef {
        input: input.to_string(),
        kind: "unknown".to_string(),
        full_ref: None,
        resolved_commit: None,
        supported_for_execute: false,
    })
}

fn verify_commitish(git: &Git, path: &Path, value: &str) -> Result<Option<String>> {
    let result = git.try_run_in(
        path,
        ["rev-parse", "--verify", &format!("{value}^{{commit}}")],
    )?;
    if result.success {
        Ok(Some(result.stdout.trim().to_string()))
    } else {
        Ok(None)
    }
}

fn looks_like_commit(value: &str) -> bool {
    value.len() >= 7 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn ref_policy(source_ref: &WorktreeSourceRef) -> WorktreeRefPolicy {
    match source_ref.kind.as_str() {
        "local_branch" => WorktreeRefPolicy {
            mode: "existing_local_branch".to_string(),
            will_create_branch: false,
            will_detach_head: false,
            will_track_upstream: false,
        },
        "tag" | "commit" => WorktreeRefPolicy {
            mode: "detached_head".to_string(),
            will_create_branch: false,
            will_detach_head: true,
            will_track_upstream: false,
        },
        "remote_tracking_branch" => WorktreeRefPolicy {
            mode: "remote_tracking_branch_blocked".to_string(),
            will_create_branch: false,
            will_detach_head: false,
            will_track_upstream: false,
        },
        _ => WorktreeRefPolicy {
            mode: "unsupported_ref".to_string(),
            will_create_branch: false,
            will_detach_head: false,
            will_track_upstream: false,
        },
    }
}

fn target_ref_name(source_ref: &WorktreeSourceRef, fallback: &str) -> String {
    match source_ref.kind.as_str() {
        "local_branch" => source_ref
            .full_ref
            .as_deref()
            .and_then(|full_ref| full_ref.strip_prefix("refs/heads/"))
            .unwrap_or(fallback)
            .to_string(),
        "tag" => source_ref
            .full_ref
            .as_deref()
            .and_then(|full_ref| full_ref.strip_prefix("refs/tags/"))
            .unwrap_or(fallback)
            .to_string(),
        "remote_tracking_branch" => source_ref
            .full_ref
            .as_deref()
            .and_then(|full_ref| full_ref.strip_prefix("refs/remotes/"))
            .unwrap_or(fallback)
            .to_string(),
        _ => fallback.to_string(),
    }
}

fn target_from_resolved(resolved: worktree_plan::ResolvedWorktreeTarget) -> WorktreeCreateTarget {
    WorktreeCreateTarget {
        path: resolved.path,
        parent: resolved.parent,
        name: resolved.name,
        ref_slug: resolved.ref_slug,
        variables: WorktreeTemplateVariablesView {
            main_path: resolved.variables.main_path,
            repo_name: resolved.variables.repo_name,
            ref_slug: resolved.variables.ref_slug,
        },
        exists: resolved.exists,
        parent_exists: resolved.parent_exists,
        parent_is_directory: resolved.parent_is_directory,
        parent_is_symlink: resolved.parent_is_symlink,
        parent_creation: WorktreeParentCreationView {
            allowed: resolved.parent_creation.allowed,
            will_create: resolved.parent_creation.will_create,
            removable_by_undo_if_empty: resolved.parent_creation.removable_by_undo_if_empty,
        },
        inside_git_dir: resolved.inside_git_dir,
        inside_existing_worktree: resolved.inside_existing_worktree,
        case_insensitive_collision: resolved.case_insensitive_collision,
        reserved_name_collision: resolved.reserved_name_collision,
    }
}

fn family_snapshot(worktrees: &[WorktreeInfo]) -> Result<WorktreeFamilySnapshot> {
    let snapshot_worktrees = worktrees
        .iter()
        .enumerate()
        .map(|(index, worktree)| WorktreeSnapshotEntry {
            path: worktree.path.clone(),
            kind: worktree_kind(index, worktree).to_string(),
            head: worktree.head.clone(),
            branch: worktree.branch.as_deref().map(full_local_branch),
            detached: worktree.detached,
            locked: false,
            prunable: false,
        })
        .collect::<Vec<_>>();
    let branch_occupancy = snapshot_worktrees
        .iter()
        .filter_map(|worktree| {
            worktree.branch.as_ref().map(|branch| BranchOccupancy {
                branch: branch.clone(),
                worktree_path: worktree.path.clone(),
            })
        })
        .collect::<Vec<_>>();

    let mut snapshot = WorktreeFamilySnapshot {
        fingerprint: String::new(),
        worktrees: snapshot_worktrees,
        branch_occupancy,
    };
    snapshot.fingerprint = family_fingerprint(&snapshot)?;
    Ok(snapshot)
}

fn worktree_kind(index: usize, worktree: &WorktreeInfo) -> &'static str {
    if worktree.bare {
        "bare"
    } else if worktree.detached {
        "detached"
    } else if index == 0 {
        "main"
    } else {
        "linked"
    }
}

fn full_local_branch(branch: &str) -> String {
    if branch.starts_with("refs/") {
        branch.to_string()
    } else {
        format!("refs/heads/{branch}")
    }
}

fn family_fingerprint(snapshot: &WorktreeFamilySnapshot) -> Result<String> {
    let input = FamilyHashInput {
        worktrees: &snapshot.worktrees,
        branch_occupancy: &snapshot.branch_occupancy,
    };
    sha256_with_domain(b"super-git-worktree-family-v0.2\n", &input)
}

fn config_used(
    app_home: &AppHome,
    settings: &WorktreeSettings,
    config: &crate::config::store::AppConfig,
) -> Result<WorktreeCreateConfigUsed> {
    Ok(WorktreeCreateConfigUsed {
        source: "global_config".to_string(),
        config_home_source: app_home.source.as_str().to_string(),
        config_fingerprint: sha256_with_domain(b"super-git-config-v0.1\n", config)?,
        worktree_template: WorktreeTemplateConfig {
            parent_template: settings.parent_template.clone(),
            name_template: settings.name_template.clone(),
            ref_slug_algorithm: settings.ref_slug_algorithm.clone(),
        },
    })
}

fn collect_source_blocks(
    source_ref: &WorktreeSourceRef,
    blocked_reasons: &mut Vec<WorktreeBlockedReason>,
) {
    match source_ref.kind.as_str() {
        "remote_tracking_branch" => blocked_reasons.push(blocked(
            "remote_tracking_branch_requires_local_branch_policy",
            json!({ "ref": source_ref.input }),
        )),
        "unknown" => blocked_reasons.push(blocked(
            "source_ref_not_found",
            json!({ "ref": source_ref.input }),
        )),
        _ => {}
    }
}

fn collect_branch_blocks(
    source_ref: &WorktreeSourceRef,
    family_snapshot: &WorktreeFamilySnapshot,
    blocked_reasons: &mut Vec<WorktreeBlockedReason>,
) {
    if source_ref.kind != "local_branch" {
        return;
    }

    let Some(full_ref) = &source_ref.full_ref else {
        return;
    };
    if let Some(occupancy) = family_snapshot
        .branch_occupancy
        .iter()
        .find(|occupancy| occupancy.branch == *full_ref)
    {
        blocked_reasons.push(blocked(
            "branch_already_checked_out",
            json!({
                "branch": occupancy.branch,
                "worktree_path": occupancy.worktree_path,
            }),
        ));
    }
}

fn collect_target_blocks(
    target: &WorktreeCreateTarget,
    blocked_reasons: &mut Vec<WorktreeBlockedReason>,
) {
    for (is_blocked, code) in [
        (target.exists, "target_path_exists"),
        (!target.parent_creation.allowed, "target_parent_not_allowed"),
        (target.inside_git_dir, "target_inside_git_dir"),
        (
            target.inside_existing_worktree,
            "target_inside_existing_worktree",
        ),
        (
            target.case_insensitive_collision,
            "target_case_insensitive_collision",
        ),
        (target.reserved_name_collision, "target_reserved_name"),
    ] {
        if is_blocked {
            blocked_reasons.push(blocked(
                code,
                json!({
                    "path": target.path,
                    "parent": target.parent,
                    "name": target.name,
                }),
            ));
        }
    }
}

fn blocked(code: &str, details: serde_json::Value) -> WorktreeBlockedReason {
    WorktreeBlockedReason {
        code: code.to_string(),
        severity: "hard_block".to_string(),
        details,
    }
}

fn preconditions(execution_status: &str) -> Vec<WorktreeCreatePrecondition> {
    let status = if execution_status == "preview_only" || execution_status == "executable" {
        "passed"
    } else {
        "blocked"
    };
    [
        "repo_family_resolved",
        "source_ref_supported",
        "branch_not_checked_out_elsewhere",
        "target_path_available",
    ]
    .into_iter()
    .map(|code| WorktreeCreatePrecondition {
        code: code.to_string(),
        status: status.to_string(),
    })
    .collect()
}

fn effects_for(execution_status: &str) -> Vec<String> {
    if execution_status == "preview_only" || execution_status == "executable" {
        vec![
            "Create one linked worktree at the resolved target path.".to_string(),
            "Check out the selected ref in the new worktree.".to_string(),
        ]
    } else {
        vec!["No write is allowed until blocked reasons are resolved.".to_string()]
    }
}

pub fn compute_worktree_plan_id(plan: &WorktreeCreatePlan) -> Result<String> {
    let execution = WorktreeExecutionHashInput {
        status: &plan.execution.status,
        super_git_execute_required: plan.execution.super_git_execute_required,
        raw_git_allowed: plan.execution.raw_git_allowed,
        blocked_reasons: &plan.execution.blocked_reasons,
    };
    let hash_input = WorktreePlanHashInput {
        schema_version: &plan.schema_version,
        action: &plan.action,
        repository: &plan.repository,
        config_used: &plan.config_used,
        source_ref: &plan.source_ref,
        ref_policy: &plan.ref_policy,
        target: &plan.target,
        family_snapshot: &plan.family_snapshot,
        preconditions: &plan.preconditions,
        execution: &execution,
        risk: &plan.risk,
        undo_strategy: &plan.undo_strategy,
    };
    sha256_with_domain(b"super-git-plan-v0.2\n", &hash_input)
}

fn sha256_with_domain<T: Serialize>(domain: &[u8], value: &T) -> Result<String> {
    let canonical = canonical_json_bytes(value)?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(canonical);
    Ok(format_digest(hasher.finalize().as_slice()))
}

fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let value = serde_json::to_value(value)?;
    serde_json::to_vec(&sort_json_value(value)).map_err(Into::into)
}

fn sort_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(sort_json_value).collect())
        }
        serde_json::Value::Object(map) => {
            let mut entries = map.into_iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));

            let mut sorted = serde_json::Map::new();
            for (key, value) in entries {
                sorted.insert(key, sort_json_value(value));
            }
            serde_json::Value::Object(sorted)
        }
        value => value,
    }
}

fn format_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity("sha256:".len() + bytes.len() * 2);
    output.push_str("sha256:");
    for byte in bytes {
        output.push(hex_char(byte >> 4));
        output.push(hex_char(byte & 0x0f));
    }
    output
}

fn hex_char(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + value - 10) as char,
        _ => unreachable!("nibble is always <= 15"),
    }
}

#[derive(Serialize)]
struct FamilyHashInput<'a> {
    worktrees: &'a [WorktreeSnapshotEntry],
    branch_occupancy: &'a [BranchOccupancy],
}

#[derive(Serialize)]
struct WorktreePlanHashInput<'a> {
    schema_version: &'a str,
    action: &'a WorktreeCreateAction,
    repository: &'a WorktreeCreateRepository,
    config_used: &'a WorktreeCreateConfigUsed,
    source_ref: &'a WorktreeSourceRef,
    ref_policy: &'a WorktreeRefPolicy,
    target: &'a WorktreeCreateTarget,
    family_snapshot: &'a WorktreeFamilySnapshot,
    preconditions: &'a [WorktreeCreatePrecondition],
    execution: &'a WorktreeExecutionHashInput<'a>,
    risk: &'a ActionRisk,
    undo_strategy: &'a WorktreeCreateUndoStrategy,
}

#[derive(Serialize)]
struct WorktreeExecutionHashInput<'a> {
    status: &'a str,
    super_git_execute_required: bool,
    raw_git_allowed: bool,
    blocked_reasons: &'a [WorktreeBlockedReason],
}

fn invalid_plan_error(error: worktree_plan::WorktreePlanError) -> SuperGitError {
    SuperGitError::PreviewPreconditionFailed {
        action: ACTION_WORKTREE_CREATE.to_string(),
        code: error.code,
        message: error.message,
    }
}

impl From<worktree_plan::WorktreePlanError> for SuperGitError {
    fn from(error: worktree_plan::WorktreePlanError) -> Self {
        invalid_plan_error(error)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::model::{
        WorktreeCreateAction, WorktreeCreateConfigUsed, WorktreeCreateExecution,
        WorktreeCreateOptions, WorktreeCreatePlan, WorktreeCreateRepository, WorktreeCreateTarget,
        WorktreeCreateUndoPreview, WorktreeCreateUndoStrategy, WorktreeFamilySnapshot,
        WorktreeParentCreationView, WorktreeRefPolicy, WorktreeReferenceCommands,
        WorktreeSourceRef, WorktreeTemplateConfig, WorktreeTemplateVariablesView,
    };

    use super::*;

    #[test]
    fn worktree_plan_id_ignores_advisory_fields() {
        let mut plan = sample_plan();
        let first = compute_worktree_plan_id(&plan).expect("plan id");

        plan.effects = vec!["Different prose.".to_string()];
        plan.reference_commands.commands = vec![vec!["git".to_string(), "status".to_string()]];
        plan.execution.suggested_super_git_command = Some(vec![
            "super-git".to_string(),
            "execute".to_string(),
            "--plan".to_string(),
            "different-file.json".to_string(),
        ]);
        plan.undo_preview.available_after_execute = false;

        let second = compute_worktree_plan_id(&plan).expect("plan id");

        assert_eq!(first, second);
    }

    fn sample_plan() -> WorktreeCreatePlan {
        WorktreeCreatePlan {
            schema_version: WORKTREE_PLAN_SCHEMA_VERSION.to_string(),
            plan_id: String::new(),
            action: WorktreeCreateAction {
                kind: ACTION_WORKTREE_CREATE.to_string(),
                options: WorktreeCreateOptions {
                    repo_selector: None,
                    ref_name: "feature/demo".to_string(),
                },
            },
            repository: WorktreeCreateRepository {
                family_id: "sha256:family".to_string(),
                kind: "worktree_family".to_string(),
                git_common_dir: PathBuf::from("/repo/.git"),
                main_worktree: Some(PathBuf::from("/repo")),
                selected_from: PathBuf::from("/repo"),
            },
            config_used: WorktreeCreateConfigUsed {
                source: "global_config".to_string(),
                config_home_source: "env:SUPER_GIT_HOME".to_string(),
                config_fingerprint: "sha256:config".to_string(),
                worktree_template: WorktreeTemplateConfig {
                    parent_template: "{main_path}.worktrees".to_string(),
                    name_template: "{repo_name}__{ref_slug}".to_string(),
                    ref_slug_algorithm: "path_safe_v1".to_string(),
                },
            },
            source_ref: WorktreeSourceRef {
                input: "feature/demo".to_string(),
                kind: "local_branch".to_string(),
                full_ref: Some("refs/heads/feature/demo".to_string()),
                resolved_commit: Some("abc123".to_string()),
                supported_for_execute: true,
            },
            ref_policy: WorktreeRefPolicy {
                mode: "existing_local_branch".to_string(),
                will_create_branch: false,
                will_detach_head: false,
                will_track_upstream: false,
            },
            target: WorktreeCreateTarget {
                path: PathBuf::from("/repo.worktrees/repo__feature-demo"),
                parent: PathBuf::from("/repo.worktrees"),
                name: "repo__feature-demo".to_string(),
                ref_slug: "feature-demo".to_string(),
                variables: WorktreeTemplateVariablesView {
                    main_path: PathBuf::from("/repo"),
                    repo_name: "repo".to_string(),
                    ref_slug: "feature-demo".to_string(),
                },
                exists: false,
                parent_exists: false,
                parent_is_directory: false,
                parent_is_symlink: false,
                parent_creation: WorktreeParentCreationView {
                    allowed: true,
                    will_create: true,
                    removable_by_undo_if_empty: true,
                },
                inside_git_dir: false,
                inside_existing_worktree: false,
                case_insensitive_collision: false,
                reserved_name_collision: false,
            },
            family_snapshot: WorktreeFamilySnapshot {
                fingerprint: "sha256:family-snapshot".to_string(),
                worktrees: Vec::new(),
                branch_occupancy: Vec::new(),
            },
            preconditions: Vec::new(),
            execution: WorktreeCreateExecution {
                status: "executable".to_string(),
                super_git_execute_required: true,
                raw_git_allowed: false,
                suggested_super_git_command: Some(vec![
                    "super-git".to_string(),
                    "execute".to_string(),
                    "--plan".to_string(),
                    "<plan-file>".to_string(),
                ]),
                blocked_reasons: Vec::new(),
            },
            risk: ActionRisk {
                severity: "medium".to_string(),
                reversibility: "reversible_if_unchanged".to_string(),
                requires_human_confirmation: false,
            },
            effects: vec!["Create worktree.".to_string()],
            reference_commands: WorktreeReferenceCommands {
                semantics: "documentation_only".to_string(),
                never_execute_directly: true,
                commands: vec![vec!["git".to_string(), "worktree".to_string()]],
            },
            undo_strategy: WorktreeCreateUndoStrategy {
                kind: "remove_created_worktree_if_clean".to_string(),
                deletes_branch: false,
                deletes_history: false,
            },
            undo_preview: WorktreeCreateUndoPreview {
                kind: "remove_created_worktree_if_clean".to_string(),
                available_after_execute: true,
                limitations: Vec::new(),
            },
        }
    }
}
