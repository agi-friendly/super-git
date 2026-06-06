use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use crate::config::store::{
    repository_id, AppConfig, SavedRepository, SavedRepositoryKind, WorktreeSettings,
};

const MAIN_PATH: &str = "main_path";
const REPO_NAME: &str = "repo_name";
const REF_SLUG: &str = "ref_slug";
const PATH_SAFE_V1: &str = "path_safe_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigValidationReport {
    pub valid: bool,
    pub issues: Vec<ConfigValidationIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigValidationIssue {
    pub field: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeTemplateUpdate {
    pub parent_template: Option<String>,
    pub name_template: Option<String>,
    pub ref_slug_algorithm: Option<String>,
}

pub fn validate_config(config: &AppConfig) -> ConfigValidationReport {
    let mut issues = Vec::new();

    validate_worktree_settings_into(&config.settings.worktree, &mut issues);
    validate_repository_registry(&config.repositories, &mut issues);

    ConfigValidationReport {
        valid: issues.is_empty(),
        issues,
    }
}

pub fn validate_worktree_settings(settings: &WorktreeSettings) -> ConfigValidationReport {
    let mut issues = Vec::new();

    validate_worktree_settings_into(settings, &mut issues);

    ConfigValidationReport {
        valid: issues.is_empty(),
        issues,
    }
}

fn validate_worktree_settings_into(
    settings: &WorktreeSettings,
    issues: &mut Vec<ConfigValidationIssue>,
) {
    validate_template(
        "settings.worktree.parent_template",
        &settings.parent_template,
        &[MAIN_PATH, REPO_NAME],
        &[MAIN_PATH],
        false,
        issues,
    );
    validate_template(
        "settings.worktree.name_template",
        &settings.name_template,
        &[REPO_NAME, REF_SLUG],
        &[REF_SLUG],
        true,
        issues,
    );
    validate_ref_slug_algorithm(&settings.ref_slug_algorithm, issues);
}

fn validate_template(
    field: &str,
    value: &str,
    allowed_variables: &[&str],
    required_variables: &[&str],
    reject_path_separators: bool,
    issues: &mut Vec<ConfigValidationIssue>,
) {
    if value.is_empty() {
        push_issue(
            issues,
            field,
            "empty_template",
            "template must not be empty",
        );
        return;
    }

    if value.contains('$') {
        push_issue(
            issues,
            field,
            "shell_variable_syntax",
            "template variables must use braces like {ref_slug}, not shell-style $VAR",
        );
    }

    if value.chars().any(char::is_control) {
        push_issue(
            issues,
            field,
            "control_character",
            "template must not contain control characters",
        );
    }

    if reject_path_separators && (value.contains('/') || value.contains('\\')) {
        push_issue(
            issues,
            field,
            "path_separator_in_name_template",
            "worktree name template must not contain path separators",
        );
    }

    let variables = collect_variables(field, value, issues);

    for variable in &variables {
        if !is_known_variable(variable) {
            push_issue(
                issues,
                field,
                "unknown_template_variable",
                &format!("unknown template variable {{{variable}}}"),
            );
            continue;
        }

        if !allowed_variables.contains(&variable.as_str()) {
            push_issue(
                issues,
                field,
                "disallowed_template_variable",
                &format!("template variable {{{variable}}} is not allowed in this field"),
            );
        }
    }

    if field == "settings.worktree.parent_template" && has_parent_traversal(value) {
        push_issue(
            issues,
            field,
            "parent_path_traversal",
            "parent template must not contain a literal .. path component",
        );
    }

    for required in required_variables {
        let count = variables
            .iter()
            .filter(|variable| variable.as_str() == *required)
            .count();

        if count == 0 {
            push_issue(
                issues,
                field,
                "missing_required_variable",
                &format!("template must include {{{required}}}"),
            );
        } else if count > 1 {
            push_issue(
                issues,
                field,
                "duplicate_required_variable",
                &format!("template must include {{{required}}} exactly once"),
            );
        }
    }
}

fn collect_variables(
    field: &str,
    value: &str,
    issues: &mut Vec<ConfigValidationIssue>,
) -> Vec<String> {
    let mut variables = Vec::new();
    let mut cursor = 0;

    while let Some(open_offset) = value[cursor..].find('{') {
        let open = cursor + open_offset;
        if value[cursor..open].contains('}') {
            push_issue(
                issues,
                field,
                "unopened_template_variable",
                "template variable is missing an opening brace",
            );
        }

        let after_open = open + 1;

        let Some(close_offset) = value[after_open..].find('}') else {
            push_issue(
                issues,
                field,
                "unclosed_template_variable",
                "template variable is missing a closing brace",
            );
            return variables;
        };

        let close = after_open + close_offset;
        let variable = &value[after_open..close];
        if variable.is_empty() {
            push_issue(
                issues,
                field,
                "empty_template_variable",
                "template variable name must not be empty",
            );
        } else {
            variables.push(variable.to_string());
        }
        cursor = close + 1;
    }

    if value[cursor..].contains('}') {
        push_issue(
            issues,
            field,
            "unopened_template_variable",
            "template variable is missing an opening brace",
        );
    }

    variables
}

fn validate_ref_slug_algorithm(algorithm: &str, issues: &mut Vec<ConfigValidationIssue>) {
    if algorithm == PATH_SAFE_V1 {
        return;
    }

    push_issue(
        issues,
        "settings.worktree.ref_slug_algorithm",
        "unsupported_ref_slug_algorithm",
        "supported ref slug algorithms: path_safe_v1",
    );
}

fn validate_repository_registry(
    repositories: &[SavedRepository],
    issues: &mut Vec<ConfigValidationIssue>,
) {
    let mut seen_ids = HashSet::new();

    for (index, repository) in repositories.iter().enumerate() {
        let field = format!("repositories[{index}]");
        validate_repository_entry(&field, repository, issues);

        if !seen_ids.insert(repository.id.clone()) {
            push_issue(
                issues,
                &format!("{field}.id"),
                "duplicate_repository_id",
                "repository id must be unique",
            );
        }
    }
}

fn validate_repository_entry(
    field: &str,
    repository: &SavedRepository,
    issues: &mut Vec<ConfigValidationIssue>,
) {
    validate_repository_id(field, repository, issues);
    validate_repository_name(field, &repository.name, issues);
    validate_repository_paths(field, repository, issues);
    validate_repository_kind(field, repository, issues);
}

fn validate_repository_id(
    field: &str,
    repository: &SavedRepository,
    issues: &mut Vec<ConfigValidationIssue>,
) {
    let id_field = format!("{field}.id");
    if !is_valid_repository_id(&repository.id) {
        push_issue(
            issues,
            &id_field,
            "invalid_repository_id",
            "repository id must be sha256:<64 lowercase hex chars>",
        );
        return;
    }

    let expected = repository_id(&repository.git_common_dir);
    if repository.id != expected {
        push_issue(
            issues,
            &id_field,
            "repository_id_mismatch",
            "repository id must match git_common_dir identity",
        );
    }
}

fn validate_repository_name(field: &str, name: &str, issues: &mut Vec<ConfigValidationIssue>) {
    let name_field = format!("{field}.name");
    if name.is_empty() {
        push_issue(
            issues,
            &name_field,
            "empty_repository_name",
            "repository name must not be empty",
        );
    }

    if name.contains('/') || name.contains('\\') {
        push_issue(
            issues,
            &name_field,
            "path_separator_in_repository_name",
            "repository name must not contain path separators",
        );
    }

    if name.chars().any(char::is_control) {
        push_issue(
            issues,
            &name_field,
            "control_character_in_repository_name",
            "repository name must not contain control characters",
        );
    }
}

fn validate_repository_paths(
    field: &str,
    repository: &SavedRepository,
    issues: &mut Vec<ConfigValidationIssue>,
) {
    if let Some(main_worktree) = &repository.main_worktree {
        validate_absolute_path(&format!("{field}.main_worktree"), main_worktree, issues);
    }
    validate_absolute_path(
        &format!("{field}.git_common_dir"),
        &repository.git_common_dir,
        issues,
    );
    validate_absolute_path(
        &format!("{field}.saved_from"),
        &repository.saved_from,
        issues,
    );
}

fn validate_absolute_path(field: &str, path: &Path, issues: &mut Vec<ConfigValidationIssue>) {
    if !path.is_absolute() {
        push_issue(
            issues,
            field,
            "repository_path_not_absolute",
            "repository path fields must be absolute",
        );
    }
}

fn validate_repository_kind(
    field: &str,
    repository: &SavedRepository,
    issues: &mut Vec<ConfigValidationIssue>,
) {
    match repository.kind {
        SavedRepositoryKind::WorktreeFamily if repository.main_worktree.is_none() => {
            push_issue(
                issues,
                &format!("{field}.main_worktree"),
                "worktree_family_missing_main_worktree",
                "worktree family entries must include main_worktree",
            );
        }
        SavedRepositoryKind::BareWorktreeFamily if repository.main_worktree.is_some() => {
            push_issue(
                issues,
                &format!("{field}.main_worktree"),
                "bare_family_has_main_worktree",
                "bare-primary worktree family entries must not include main_worktree",
            );
        }
        _ => {}
    }
}

fn is_valid_repository_id(id: &str) -> bool {
    let Some(hex) = id.strip_prefix("sha256:") else {
        return false;
    };

    hex.len() == 64
        && hex
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
}

fn has_parent_traversal(value: &str) -> bool {
    value.split(['/', '\\']).any(|component| component == "..")
}

fn is_known_variable(variable: &str) -> bool {
    matches!(variable, MAIN_PATH | REPO_NAME | REF_SLUG)
}

fn push_issue(issues: &mut Vec<ConfigValidationIssue>, field: &str, code: &str, message: &str) {
    issues.push(ConfigValidationIssue {
        field: field.to_string(),
        code: code.to_string(),
        message: message.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::config::store::{SavedRepository, SavedRepositoryKind};

    use super::*;

    fn valid_repository(id: &str, name: &str) -> SavedRepository {
        SavedRepository {
            id: id.to_string(),
            name: name.to_string(),
            kind: SavedRepositoryKind::WorktreeFamily,
            main_worktree: Some(PathBuf::from("/repo/main")),
            git_common_dir: PathBuf::from("/repo/main/.git"),
            saved_from: PathBuf::from("/repo/main"),
        }
    }

    #[test]
    fn validates_default_worktree_settings() {
        let report = validate_worktree_settings(&WorktreeSettings::default());

        assert!(report.valid);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn rejects_unknown_variables() {
        let settings = WorktreeSettings {
            name_template: "{repo_name}__{branch}".to_string(),
            ..WorktreeSettings::default()
        };

        let report = validate_worktree_settings(&settings);

        assert!(!report.valid);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "unknown_template_variable"));
    }

    #[test]
    fn rejects_shell_style_variables() {
        let settings = WorktreeSettings {
            name_template: "$REPO_NAME__$REF_SLUG".to_string(),
            ..WorktreeSettings::default()
        };

        let report = validate_worktree_settings(&settings);

        assert!(!report.valid);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "shell_variable_syntax"));
    }

    #[test]
    fn rejects_empty_template_variables() {
        let settings = WorktreeSettings {
            name_template: "{repo_name}__{}".to_string(),
            ..WorktreeSettings::default()
        };

        let report = validate_worktree_settings(&settings);

        assert!(!report.valid);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "empty_template_variable"));
    }

    #[test]
    fn rejects_unmatched_template_braces() {
        let settings = WorktreeSettings {
            parent_template: "oops}{main_path}".to_string(),
            name_template: "{repo_name}__{ref_slug".to_string(),
            ..WorktreeSettings::default()
        };

        let report = validate_worktree_settings(&settings);

        assert!(!report.valid);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "unopened_template_variable"));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "unclosed_template_variable"));
    }

    #[test]
    fn rejects_control_characters() {
        let settings = WorktreeSettings {
            name_template: "{repo_name}__{ref_slug}\n".to_string(),
            ..WorktreeSettings::default()
        };

        let report = validate_worktree_settings(&settings);

        assert!(!report.valid);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "control_character"));
    }

    #[test]
    fn rejects_disallowed_variables_for_each_field() {
        let settings = WorktreeSettings {
            parent_template: "{main_path}/{ref_slug}".to_string(),
            name_template: "{main_path}__{ref_slug}".to_string(),
            ..WorktreeSettings::default()
        };

        let report = validate_worktree_settings(&settings);

        assert!(!report.valid);
        assert_eq!(
            report
                .issues
                .iter()
                .filter(|issue| issue.code == "disallowed_template_variable")
                .count(),
            2
        );
    }

    #[test]
    fn rejects_parent_templates_without_main_path() {
        let settings = WorktreeSettings {
            parent_template: "{repo_name}.worktrees".to_string(),
            ..WorktreeSettings::default()
        };

        let report = validate_worktree_settings(&settings);

        assert!(!report.valid);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "missing_required_variable"));
    }

    #[test]
    fn rejects_duplicate_required_variables() {
        let settings = WorktreeSettings {
            parent_template: "{main_path}/{main_path}.worktrees".to_string(),
            ..WorktreeSettings::default()
        };

        let report = validate_worktree_settings(&settings);

        assert!(!report.valid);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "duplicate_required_variable"));
    }

    #[test]
    fn rejects_name_templates_without_ref_slug() {
        let settings = WorktreeSettings {
            name_template: "{repo_name}".to_string(),
            ..WorktreeSettings::default()
        };

        let report = validate_worktree_settings(&settings);

        assert!(!report.valid);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "missing_required_variable"));
    }

    #[test]
    fn rejects_path_separators_in_name_template() {
        let settings = WorktreeSettings {
            name_template: "{repo_name}/{ref_slug}".to_string(),
            ..WorktreeSettings::default()
        };

        let report = validate_worktree_settings(&settings);

        assert!(!report.valid);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "path_separator_in_name_template"));
    }

    #[test]
    fn rejects_parent_traversal_components() {
        let settings = WorktreeSettings {
            parent_template: "{main_path}/../worktrees".to_string(),
            ..WorktreeSettings::default()
        };

        let report = validate_worktree_settings(&settings);

        assert!(!report.valid);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "parent_path_traversal"));
    }

    #[test]
    fn rejects_unsupported_ref_slug_algorithm() {
        let settings = WorktreeSettings {
            ref_slug_algorithm: "raw".to_string(),
            ..WorktreeSettings::default()
        };

        let report = validate_worktree_settings(&settings);

        assert!(!report.valid);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "unsupported_ref_slug_algorithm"));
    }

    #[test]
    fn rejects_invalid_repository_registry_entries() {
        let mut config = AppConfig::default();
        config.repositories.push(SavedRepository {
            id: "not-a-sha".to_string(),
            name: "bad/name".to_string(),
            kind: SavedRepositoryKind::BareWorktreeFamily,
            main_worktree: Some(PathBuf::from("relative-main")),
            git_common_dir: PathBuf::from("relative-git"),
            saved_from: PathBuf::from("relative-saved"),
        });

        let report = validate_config(&config);

        assert!(!report.valid);
        for expected_code in [
            "invalid_repository_id",
            "path_separator_in_repository_name",
            "repository_path_not_absolute",
            "bare_family_has_main_worktree",
        ] {
            assert!(
                report
                    .issues
                    .iter()
                    .any(|issue| issue.code == expected_code),
                "missing issue code {expected_code}: {:?}",
                report.issues
            );
        }
    }

    #[test]
    fn rejects_duplicate_repository_ids() {
        let duplicate_id = format!("sha256:{}", "a".repeat(64));
        let mut config = AppConfig::default();
        config
            .repositories
            .push(valid_repository(&duplicate_id, "one"));
        config
            .repositories
            .push(valid_repository(&duplicate_id, "two"));

        let report = validate_config(&config);

        assert!(!report.valid);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "duplicate_repository_id"));
    }
}
