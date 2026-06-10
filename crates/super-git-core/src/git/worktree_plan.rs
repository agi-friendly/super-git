use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::store::{SavedRepository, WorktreeSettings};
use crate::model::WorktreeInfo;

const PATH_SAFE_V1: &str = "path_safe_v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreePlanError {
    pub code: String,
    pub message: String,
}

impl fmt::Display for WorktreePlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for WorktreePlanError {}

pub type WorktreePlanResult<T> = std::result::Result<T, WorktreePlanError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedWorktreeTarget {
    pub ref_slug: String,
    pub parent: PathBuf,
    pub name: String,
    pub path: PathBuf,
    pub variables: WorktreeTemplateVariables,
    pub exists: bool,
    pub parent_exists: bool,
    pub parent_is_directory: bool,
    pub parent_is_symlink: bool,
    pub parent_creation: ParentCreation,
    pub inside_git_dir: bool,
    pub inside_existing_worktree: bool,
    pub case_insensitive_collision: bool,
    pub reserved_name_collision: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeTemplateVariables {
    pub main_path: PathBuf,
    pub repo_name: String,
    pub ref_slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParentCreation {
    pub allowed: bool,
    pub will_create: bool,
    pub removable_by_undo_if_empty: bool,
}

pub fn path_safe_v1(input: &str) -> WorktreePlanResult<String> {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for character in input.chars() {
        if should_replace_with_dash(character) {
            if !last_was_dash {
                slug.push('-');
                last_was_dash = true;
            }
            continue;
        }

        slug.push(character);
        last_was_dash = false;
    }

    let slug = slug.trim_end_matches(['.', ' ']).to_string();
    if slug.is_empty() {
        return Err(plan_error(
            "empty_ref_slug",
            "ref slug is empty after path_safe_v1 normalization",
        ));
    }

    if is_windows_reserved_component(&slug) {
        Ok(format!("ref-{slug}"))
    } else {
        Ok(slug)
    }
}

pub fn resolve_worktree_target(
    repository: &SavedRepository,
    settings: &WorktreeSettings,
    ref_name: &str,
    existing_worktrees: &[WorktreeInfo],
    case_insensitive_fs: bool,
) -> WorktreePlanResult<ResolvedWorktreeTarget> {
    if settings.ref_slug_algorithm != PATH_SAFE_V1 {
        return Err(plan_error(
            "unsupported_ref_slug_algorithm",
            "supported ref slug algorithms: path_safe_v1",
        ));
    }

    let Some(main_path) = repository.main_worktree.clone() else {
        return Err(plan_error(
            "main_worktree_required",
            "default worktree templates require a repository family with a main worktree",
        ));
    };

    let ref_slug = path_safe_v1(ref_name)?;
    let variables = WorktreeTemplateVariables {
        main_path: main_path.clone(),
        repo_name: repository.name.clone(),
        ref_slug,
    };
    let parent = render_path_template(&settings.parent_template, &variables);
    let name = render_string_template(&settings.name_template, &variables);
    let path = parent.join(&name);

    let exists = path.exists();
    let parent_exists = parent.exists();
    let parent_is_directory = parent.is_dir();
    let parent_is_symlink = is_symlink(&parent);
    let inside_git_dir = is_inside(&path, &repository.git_common_dir);
    let inside_existing_worktree = existing_worktrees
        .iter()
        .filter(|worktree| !worktree.bare)
        .any(|worktree| is_inside(&path, &worktree.path));
    let case_insensitive_collision =
        has_case_insensitive_collision(&name, &parent, existing_worktrees, case_insensitive_fs);
    let reserved_name_collision = is_windows_reserved_component(&name);
    let parent_creation = parent_creation_policy(
        &parent,
        parent_exists,
        parent_is_directory,
        parent_is_symlink,
        &repository.git_common_dir,
        existing_worktrees,
    );

    Ok(ResolvedWorktreeTarget {
        ref_slug: variables.ref_slug.clone(),
        parent,
        name,
        path,
        variables,
        exists,
        parent_exists,
        parent_is_directory,
        parent_is_symlink,
        parent_creation,
        inside_git_dir,
        inside_existing_worktree,
        case_insensitive_collision,
        reserved_name_collision,
    })
}

fn should_replace_with_dash(character: char) -> bool {
    matches!(
        character,
        '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'
    ) || character.is_control()
}

fn render_path_template(template: &str, variables: &WorktreeTemplateVariables) -> PathBuf {
    PathBuf::from(render_string_template(template, variables))
}

fn render_string_template(template: &str, variables: &WorktreeTemplateVariables) -> String {
    template
        .replace("{main_path}", &variables.main_path.to_string_lossy())
        .replace("{repo_name}", &variables.repo_name)
        .replace("{ref_slug}", &variables.ref_slug)
}

fn is_symlink(path: &Path) -> bool {
    path.symlink_metadata()
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn is_inside(path: &Path, parent: &Path) -> bool {
    path == parent || path.starts_with(parent)
}

/// Shared by preview (resolve_worktree_target) and execute-side revalidation so
/// the two never disagree on whether a target collides.
pub fn has_case_insensitive_collision(
    target_name: &str,
    parent: &Path,
    existing_worktrees: &[WorktreeInfo],
    case_insensitive_fs: bool,
) -> bool {
    // On a case-sensitive filesystem, names that differ only by case coexist
    // fine, so flagging them would over-block legitimate targets. Gate the check
    // on git's own core.ignorecase detection. (A normalization-insensitive
    // collision -- e.g. NFC vs NFD on macOS APFS -- is still missed; folding
    // those would require Unicode normalization and a new dependency.)
    if !case_insensitive_fs {
        return false;
    }
    let target = target_name.to_lowercase();

    if existing_worktrees.iter().any(|worktree| {
        worktree
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_lowercase() == target)
    }) {
        return true;
    }

    let Ok(entries) = std::fs::read_dir(parent) else {
        return false;
    };

    entries.filter_map(std::result::Result::ok).any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.to_lowercase() == target)
    })
}

fn parent_creation_policy(
    parent: &Path,
    parent_exists: bool,
    parent_is_directory: bool,
    parent_is_symlink: bool,
    git_common_dir: &Path,
    existing_worktrees: &[WorktreeInfo],
) -> ParentCreation {
    if parent_exists {
        let allowed = parent_is_directory && !parent_is_symlink;
        return ParentCreation {
            allowed,
            will_create: false,
            removable_by_undo_if_empty: false,
        };
    }

    let own_parent_exists = parent.parent().is_some_and(Path::is_dir);
    let parent_inside_git_dir = is_inside(parent, git_common_dir);
    let parent_inside_existing_worktree = existing_worktrees
        .iter()
        .filter(|worktree| !worktree.bare)
        .any(|worktree| is_inside(parent, &worktree.path));
    let allowed = own_parent_exists && !parent_inside_git_dir && !parent_inside_existing_worktree;

    ParentCreation {
        allowed,
        will_create: allowed,
        removable_by_undo_if_empty: allowed,
    }
}

fn is_windows_reserved_component(value: &str) -> bool {
    let basename = value.split('.').next().unwrap_or(value);
    let uppercase = basename.to_ascii_uppercase();

    matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || is_numbered_reserved(&uppercase, "COM")
        || is_numbered_reserved(&uppercase, "LPT")
}

fn is_numbered_reserved(value: &str, prefix: &str) -> bool {
    let Some(suffix) = value.strip_prefix(prefix) else {
        return false;
    };

    matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
}

fn plan_error(code: &str, message: &str) -> WorktreePlanError {
    WorktreePlanError {
        code: code.to_string(),
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::config::store::{SavedRepository, SavedRepositoryKind, WorktreeSettings};
    use crate::model::WorktreeInfo;

    use super::*;

    fn saved_repository(name: &str, main_worktree: Option<PathBuf>) -> SavedRepository {
        let git_common_dir = main_worktree
            .as_ref()
            .map(|path| path.join(".git"))
            .unwrap_or_else(|| PathBuf::from("/tmp/bare.git"));

        SavedRepository {
            id: "sha256:test".to_string(),
            name: name.to_string(),
            kind: if main_worktree.is_some() {
                SavedRepositoryKind::WorktreeFamily
            } else {
                SavedRepositoryKind::BareWorktreeFamily
            },
            main_worktree: main_worktree.clone(),
            git_common_dir,
            saved_from: main_worktree.unwrap_or_else(|| PathBuf::from("/tmp/linked")),
        }
    }

    #[test]
    fn path_safe_v1_replaces_invalid_runs_with_single_dash() {
        assert_eq!(
            path_safe_v1("feature//foo??bar").expect("slug"),
            "feature-foo-bar"
        );
        assert_eq!(
            path_safe_v1(r"release\\2026:Q2").expect("slug"),
            "release-2026-Q2"
        );
    }

    #[test]
    fn path_safe_v1_prefixes_windows_reserved_names() {
        assert_eq!(path_safe_v1("CON.txt").expect("slug"), "ref-CON.txt");
        assert_eq!(path_safe_v1("aux").expect("slug"), "ref-aux");
    }

    #[test]
    fn path_safe_v1_rejects_empty_results() {
        let error = path_safe_v1("... ").expect_err("empty slug");

        assert_eq!(error.code, "empty_ref_slug");
    }

    #[test]
    fn resolves_default_target_from_repository_settings_and_ref() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let main = temp_dir.path().join("naon-dnl");
        std::fs::create_dir(&main).expect("main dir");
        let repository = saved_repository("naon-dnl", Some(main.clone()));
        let worktrees = vec![WorktreeInfo::new(main.clone())];

        let target = resolve_worktree_target(
            &repository,
            &WorktreeSettings::default(),
            "works/eml-base",
            &worktrees,
            false,
        )
        .expect("resolve target");

        assert_eq!(target.ref_slug, "works-eml-base");
        assert_eq!(target.parent, temp_dir.path().join("naon-dnl.worktrees"));
        assert_eq!(target.name, "naon-dnl__works-eml-base");
        assert_eq!(
            target.path,
            temp_dir
                .path()
                .join("naon-dnl.worktrees")
                .join("naon-dnl__works-eml-base")
        );
        assert!(!target.exists);
        assert!(!target.parent_exists);
        assert!(target.parent_creation.allowed);
        assert!(target.parent_creation.will_create);
        assert!(target.parent_creation.removable_by_undo_if_empty);
    }

    #[test]
    fn resolver_detects_case_insensitive_target_name_collisions() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let main = temp_dir.path().join("naon-dnl");
        let parent = temp_dir.path().join("naon-dnl.worktrees");
        std::fs::create_dir(&main).expect("main dir");
        std::fs::create_dir(&parent).expect("parent dir");
        std::fs::create_dir(parent.join("naon-dnl__Works-EML-Base")).expect("existing target");
        let repository = saved_repository("naon-dnl", Some(main.clone()));
        let worktrees = vec![WorktreeInfo::new(main)];

        let target = resolve_worktree_target(
            &repository,
            &WorktreeSettings::default(),
            "works/eml-base",
            &worktrees,
            true,
        )
        .expect("resolve target");

        assert!(target.case_insensitive_collision);
    }

    #[test]
    fn resolver_skips_case_collision_on_case_sensitive_fs() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let main = temp_dir.path().join("naon-dnl");
        let parent = temp_dir.path().join("naon-dnl.worktrees");
        std::fs::create_dir(&main).expect("main dir");
        std::fs::create_dir(&parent).expect("parent dir");
        std::fs::create_dir(parent.join("naon-dnl__Works-EML-Base")).expect("existing target");
        let repository = saved_repository("naon-dnl", Some(main.clone()));
        let worktrees = vec![WorktreeInfo::new(main)];

        // case_insensitive_fs = false: differently-cased names coexist, so no
        // collision should be flagged (the same fixture flags one when true).
        let target = resolve_worktree_target(
            &repository,
            &WorktreeSettings::default(),
            "works/eml-base",
            &worktrees,
            false,
        )
        .expect("resolve target");

        assert!(!target.case_insensitive_collision);
    }

    #[test]
    fn resolver_marks_targets_inside_git_dir_and_existing_worktree() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let main = temp_dir.path().join("repo");
        let git_dir = main.join(".git");
        std::fs::create_dir(&main).expect("main dir");
        std::fs::create_dir(&git_dir).expect("git dir");
        let repository = saved_repository("repo", Some(main.clone()));
        let worktrees = vec![WorktreeInfo::new(main.clone())];
        let settings = WorktreeSettings {
            parent_template: "{main_path}/.git/worktrees".to_string(),
            name_template: "{repo_name}__{ref_slug}".to_string(),
            ref_slug_algorithm: "path_safe_v1".to_string(),
        };

        let target =
            resolve_worktree_target(&repository, &settings, "feature/demo", &worktrees, false)
                .expect("resolve target");

        assert!(target.inside_git_dir);
        assert!(target.inside_existing_worktree);
    }

    #[test]
    fn resolver_rejects_default_template_for_bare_primary_families() {
        let repository = saved_repository("bare", None);
        let error = resolve_worktree_target(
            &repository,
            &WorktreeSettings::default(),
            "feature/demo",
            &[],
            false,
        )
        .expect_err("bare primary requires explicit parent support");

        assert_eq!(error.code, "main_worktree_required");
    }
}
