use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::config::template::{
    validate_config as validate_app_config, ConfigValidationReport, WorktreeTemplateUpdate,
};
use crate::git::command::Git;
use crate::git::repository::validate_repository_path;
use crate::git::worktree;
use crate::{Result, SuperGitError};

pub const SUPER_GIT_HOME_ENV: &str = "SUPER_GIT_HOME";
pub const CONFIG_SCHEMA_VERSION: u32 = 1;
const CONFIG_FILE_NAME: &str = "config.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "current_config_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub settings: ConfigSettings,
    #[serde(default)]
    pub repositories: Vec<SavedRepository>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            settings: ConfigSettings::default(),
            repositories: Vec::new(),
        }
    }
}

impl AppConfig {
    fn from_repositories(repositories: Vec<SavedRepository>) -> Self {
        Self {
            repositories,
            ..Self::default()
        }
    }

    pub fn add_repository(&mut self, repository: SavedRepository) -> bool {
        self.add_or_get_repository(repository).1
    }

    pub fn add_or_get_repository(
        &mut self,
        repository: SavedRepository,
    ) -> (SavedRepository, bool) {
        if let Some(existing) = self
            .repositories
            .iter()
            .find(|repo| repo.id == repository.id)
        {
            return (existing.clone(), false);
        }

        self.repositories.push(repository.clone());
        (repository, true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConfigSettings {
    #[serde(default)]
    pub worktree: WorktreeSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeSettings {
    #[serde(default = "default_worktree_parent_template")]
    pub parent_template: String,
    #[serde(default = "default_worktree_name_template")]
    pub name_template: String,
    #[serde(default = "default_worktree_ref_slug_algorithm")]
    pub ref_slug_algorithm: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRepository {
    pub id: String,
    pub name: String,
    pub kind: SavedRepositoryKind,
    pub main_worktree: Option<PathBuf>,
    pub git_common_dir: PathBuf,
    pub saved_from: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedRepositoryKind {
    WorktreeFamily,
    BareWorktreeFamily,
}

impl Default for WorktreeSettings {
    fn default() -> Self {
        Self {
            parent_template: default_worktree_parent_template(),
            name_template: default_worktree_name_template(),
            ref_slug_algorithm: default_worktree_ref_slug_algorithm(),
        }
    }
}

impl SavedRepositoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SavedRepositoryKind::WorktreeFamily => "worktree_family",
            SavedRepositoryKind::BareWorktreeFamily => "bare_worktree_family",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppHome {
    pub home: PathBuf,
    pub source: AppHomeSource,
    pub config_file: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AppHomeSource {
    #[serde(rename = "env:SUPER_GIT_HOME")]
    EnvSuperGitHome,
    #[serde(rename = "project_dirs")]
    ProjectDirs,
}

impl AppHomeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            AppHomeSource::EnvSuperGitHome => "env:SUPER_GIT_HOME",
            AppHomeSource::ProjectDirs => "project_dirs",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveRepositoryResult {
    pub repository: SavedRepository,
    pub requested_path: PathBuf,
    pub added: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgetRepositoryResult {
    pub target: String,
    pub repository: SavedRepository,
    pub removed: bool,
    pub matched_by: RepositoryTargetMatch,
    pub remaining_repositories: usize,
    pub registry_only: bool,
    pub filesystem_deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryTargetMatch {
    Id,
    Path,
    Name,
}

impl RepositoryTargetMatch {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Path => "path",
            Self::Name => "name",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSavedRepository {
    pub repository: SavedRepository,
    pub matched_by: RepositoryTargetMatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigUpdateResult {
    pub config: AppConfig,
    pub changed: bool,
    pub validation: ConfigValidationReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LoadedConfig {
    config: AppConfig,
    needs_save: bool,
}

impl ConfigStore {
    pub fn from_default_path() -> Result<Self> {
        Ok(Self {
            path: default_config_path()?,
        })
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<AppConfig> {
        Ok(self.load_with_metadata()?.config)
    }

    pub fn resolve_saved_repository(&self, target: &str) -> Result<ResolvedSavedRepository> {
        let config = self.load()?;
        resolve_repository_in_config(&config, target)
    }

    fn load_with_metadata(&self) -> Result<LoadedConfig> {
        if !self.path.exists() {
            return Ok(LoadedConfig {
                config: AppConfig::default(),
                needs_save: false,
            });
        }

        let content = fs::read_to_string(&self.path)?;
        parse_config(&content)
    }

    pub fn save(&self, config: &AppConfig) -> Result<()> {
        let parent = config_parent_dir(&self.path);

        if let Some(parent) = parent {
            fs::create_dir_all(parent)?;
        }

        let mut config = config.clone();
        config.schema_version = CONFIG_SCHEMA_VERSION;
        let content = serde_json::to_string_pretty(&config)?;
        let mut temp_file = NamedTempFile::new_in(parent.unwrap_or_else(|| Path::new(".")))?;

        writeln!(temp_file, "{content}")?;
        temp_file.as_file_mut().sync_all()?;
        temp_file.persist(&self.path).map_err(|err| err.error)?;
        Ok(())
    }

    pub fn save_repository(&self, path: &Path) -> Result<SaveRepositoryResult> {
        let requested_path = validate_repository_path(path)?;
        let candidate = SavedRepository::from_path(&requested_path)?;
        let mut loaded = self.load_with_metadata()?;
        let (repository, added) = loaded.config.add_or_get_repository(candidate);

        if added || loaded.needs_save {
            self.save(&loaded.config)?;
        }

        Ok(SaveRepositoryResult {
            repository,
            requested_path,
            added,
        })
    }

    pub fn forget_repository(&self, target: &str) -> Result<ForgetRepositoryResult> {
        let mut loaded = self.load_with_metadata()?;
        let resolved = resolve_repository_target(&loaded.config.repositories, target)?;
        let repository = loaded.config.repositories.remove(resolved.index);
        let remaining_repositories = loaded.config.repositories.len();

        self.save(&loaded.config)?;

        Ok(ForgetRepositoryResult {
            target: target.to_string(),
            repository,
            removed: true,
            matched_by: resolved.matched_by,
            remaining_repositories,
            registry_only: true,
            filesystem_deleted: false,
        })
    }

    pub fn validate(&self) -> Result<ConfigValidationReport> {
        let config = self.load()?;
        Ok(validate_app_config(&config))
    }

    pub fn set_worktree_template(
        &self,
        update: WorktreeTemplateUpdate,
    ) -> Result<ConfigUpdateResult> {
        let file_exists = self.path.exists();
        let mut loaded = self.load_with_metadata()?;
        let before = loaded.config.settings.worktree.clone();

        if let Some(parent_template) = update.parent_template {
            loaded.config.settings.worktree.parent_template = parent_template;
        }
        if let Some(name_template) = update.name_template {
            loaded.config.settings.worktree.name_template = name_template;
        }
        if let Some(ref_slug_algorithm) = update.ref_slug_algorithm {
            loaded.config.settings.worktree.ref_slug_algorithm = ref_slug_algorithm;
        }

        let validation = validate_app_config(&loaded.config);
        if let Some(issue) = validation.issues.first() {
            return Err(SuperGitError::ConfigValidationFailed {
                field: issue.field.clone(),
                code: issue.code.clone(),
                message: issue.message.clone(),
            });
        }

        let changed =
            !file_exists || loaded.needs_save || loaded.config.settings.worktree != before;
        if changed {
            self.save(&loaded.config)?;
        }

        Ok(ConfigUpdateResult {
            config: loaded.config,
            changed,
            validation,
        })
    }
}

#[derive(Debug, Deserialize)]
struct LegacyAppConfig {
    #[serde(default)]
    repositories: Vec<LegacyRepository>,
}

#[derive(Debug, Deserialize)]
struct ConfigFile {
    #[serde(default = "current_config_schema_version")]
    schema_version: u32,
    #[serde(default)]
    settings: ConfigSettings,
    #[serde(default)]
    repositories: Vec<RepositoryFileEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RepositoryFileEntry {
    Current(SavedRepository),
    Legacy(LegacyRepository),
}

#[derive(Debug, Deserialize)]
struct LegacyRepository {
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepositoryEntries {
    repositories: Vec<SavedRepository>,
    needs_save: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedRepositoryTarget {
    index: usize,
    matched_by: RepositoryTargetMatch,
}

fn parse_config(content: &str) -> Result<LoadedConfig> {
    let value: serde_json::Value = serde_json::from_str(content)?;

    match value.get("schema_version") {
        None => {
            let legacy: LegacyAppConfig = serde_json::from_value(value)?;
            let entries = repository_entries_from_legacy(legacy.repositories)?;
            Ok(LoadedConfig {
                config: AppConfig::from_repositories(entries.repositories),
                needs_save: true,
            })
        }
        Some(version) => {
            let version = version
                .as_u64()
                .ok_or_else(|| SuperGitError::InvalidConfigSchemaVersion(version.to_string()))?;

            if version != u64::from(CONFIG_SCHEMA_VERSION) {
                return Err(SuperGitError::UnsupportedConfigSchemaVersion {
                    version,
                    current: CONFIG_SCHEMA_VERSION,
                });
            }

            let config: ConfigFile = serde_json::from_value(value)?;
            let entries = repository_entries_from_file(config.repositories)?;
            let config = AppConfig {
                schema_version: config.schema_version,
                settings: config.settings,
                repositories: entries.repositories,
            };

            Ok(LoadedConfig {
                config,
                needs_save: entries.needs_save,
            })
        }
    }
}

fn repository_entries_from_file(entries: Vec<RepositoryFileEntry>) -> Result<RepositoryEntries> {
    let mut config = AppConfig::default();
    let mut needs_save = false;

    for entry in entries {
        match entry {
            RepositoryFileEntry::Current(repository) => {
                config.repositories.push(repository);
            }
            RepositoryFileEntry::Legacy(repository) => {
                needs_save = true;
                if let Some(repository) = migrate_legacy_repository(repository)? {
                    config.add_repository(repository);
                }
            }
        }
    }

    Ok(RepositoryEntries {
        repositories: config.repositories,
        needs_save,
    })
}

fn repository_entries_from_legacy(entries: Vec<LegacyRepository>) -> Result<RepositoryEntries> {
    let mut config = AppConfig::default();

    for entry in entries {
        if let Some(repository) = migrate_legacy_repository(entry)? {
            config.add_repository(repository);
        }
    }

    Ok(RepositoryEntries {
        repositories: config.repositories,
        needs_save: true,
    })
}

fn resolve_repository_target(
    repositories: &[SavedRepository],
    target: &str,
) -> Result<ResolvedRepositoryTarget> {
    let mut matches = Vec::new();
    for (indexes, matched_by) in [
        (
            indexes_matching_id(repositories, target),
            RepositoryTargetMatch::Id,
        ),
        (
            indexes_matching_path(repositories, target),
            RepositoryTargetMatch::Path,
        ),
        (
            indexes_matching_name(repositories, target),
            RepositoryTargetMatch::Name,
        ),
    ] {
        for index in indexes {
            matches.push((index, matched_by));
        }
    }

    let unique_indexes = unique_match_indexes(&matches);
    match unique_indexes.len() {
        0 => {}
        1 => {
            let index = unique_indexes[0];
            let matched_by = matches
                .iter()
                .find_map(|(candidate_index, matched_by)| {
                    (*candidate_index == index).then_some(*matched_by)
                })
                .expect("unique index must have a match kind");

            return Ok(ResolvedRepositoryTarget { index, matched_by });
        }
        _ => {
            return Err(SuperGitError::AmbiguousRepositoryTarget {
                target: target.to_string(),
                matches: unique_indexes
                    .iter()
                    .map(|index| repository_match_label(&repositories[*index]))
                    .collect(),
            });
        }
    }

    Err(SuperGitError::RepositoryNotFound {
        target: target.to_string(),
    })
}

pub fn resolve_repository_in_config(
    config: &AppConfig,
    target: &str,
) -> Result<ResolvedSavedRepository> {
    let resolved = resolve_repository_target(&config.repositories, target)?;
    Ok(ResolvedSavedRepository {
        repository: config.repositories[resolved.index].clone(),
        matched_by: resolved.matched_by,
    })
}

fn unique_match_indexes(matches: &[(usize, RepositoryTargetMatch)]) -> Vec<usize> {
    let mut indexes = Vec::new();
    for (index, _) in matches {
        if !indexes.contains(index) {
            indexes.push(*index);
        }
    }

    indexes
}

fn indexes_matching_id(repositories: &[SavedRepository], target: &str) -> Vec<usize> {
    repositories
        .iter()
        .enumerate()
        .filter_map(|(index, repository)| (repository.id == target).then_some(index))
        .collect()
}

fn indexes_matching_path(repositories: &[SavedRepository], target: &str) -> Vec<usize> {
    if !looks_like_path(target) {
        return Vec::new();
    }

    let target_path = Path::new(target);
    let canonical_target = std::fs::canonicalize(target_path).ok();
    let target_family_id = SavedRepository::from_path(target_path)
        .ok()
        .map(|repository| repository.id);

    repositories
        .iter()
        .enumerate()
        .filter_map(|(index, repository)| {
            let direct_match = repository_matches_path(repository, target_path);
            let canonical_match = canonical_target
                .as_deref()
                .is_some_and(|path| repository_matches_path(repository, path));
            let family_match = target_family_id
                .as_ref()
                .is_some_and(|id| repository.id == *id);

            (direct_match || canonical_match || family_match).then_some(index)
        })
        .collect()
}

fn looks_like_path(target: &str) -> bool {
    target == "."
        || target.contains('/')
        || target.contains('\\')
        || Path::new(target).is_absolute()
}

fn indexes_matching_name(repositories: &[SavedRepository], target: &str) -> Vec<usize> {
    repositories
        .iter()
        .enumerate()
        .filter_map(|(index, repository)| (repository.name == target).then_some(index))
        .collect()
}

fn repository_matches_path(repository: &SavedRepository, path: &Path) -> bool {
    repository.main_worktree.as_deref() == Some(path)
        || repository.git_common_dir == path
        || repository.saved_from == path
}

fn repository_match_label(repository: &SavedRepository) -> String {
    format!("{} ({})", repository.name, repository.id)
}

fn migrate_legacy_repository(entry: LegacyRepository) -> Result<Option<SavedRepository>> {
    match SavedRepository::from_path(&entry.path) {
        Ok(repository) => Ok(Some(repository)),
        Err(err) if is_stale_legacy_repository_error(&err) => Ok(None),
        Err(err) => Err(err),
    }
}

fn is_stale_legacy_repository_error(err: &SuperGitError) -> bool {
    matches!(
        err,
        SuperGitError::PathDoesNotExist(_)
            | SuperGitError::PathIsNotDirectory(_)
            | SuperGitError::NotGitRepository(_)
    )
}

pub fn default_config_path() -> Result<PathBuf> {
    Ok(default_app_home()?.config_file)
}

pub fn default_app_home() -> Result<AppHome> {
    if let Some(home) = std::env::var_os(SUPER_GIT_HOME_ENV) {
        if home.is_empty() {
            return Err(SuperGitError::EmptySuperGitHome);
        }

        let home = PathBuf::from(home);
        if !home.is_absolute() {
            return Err(SuperGitError::RelativeSuperGitHome(home));
        }

        return Ok(AppHome::from_home(home, AppHomeSource::EnvSuperGitHome));
    }

    let dirs = ProjectDirs::from("com", "super-git", "super-git")
        .ok_or(SuperGitError::ConfigDirectoryUnavailable)?;

    Ok(AppHome::from_home(
        dirs.config_dir().to_path_buf(),
        AppHomeSource::ProjectDirs,
    ))
}

impl AppHome {
    pub fn from_home(home: PathBuf, source: AppHomeSource) -> Self {
        let config_file = home.join(CONFIG_FILE_NAME);

        Self {
            home,
            source,
            config_file,
        }
    }
}

impl SavedRepository {
    pub fn from_path(path: &Path) -> Result<Self> {
        let path = validate_repository_path(path)?;
        let git = Git::default();
        let saved_from = repository_root(&git, &path)?;
        let git_common_dir = git_common_dir(&git, &path)?;
        let worktrees = worktree::list_worktrees(&path)?;
        let main_worktree = worktrees
            .first()
            .filter(|worktree| !worktree.bare)
            .map(|worktree| normalize_path(&worktree.path))
            .transpose()?;
        let kind = if main_worktree.is_some() {
            SavedRepositoryKind::WorktreeFamily
        } else {
            SavedRepositoryKind::BareWorktreeFamily
        };
        let name = repository_name(main_worktree.as_ref(), &git_common_dir);
        let id = repository_id(&git_common_dir);

        Ok(Self {
            id,
            name,
            kind,
            main_worktree,
            git_common_dir,
            saved_from,
        })
    }
}

fn repository_root(git: &Git, path: &Path) -> Result<PathBuf> {
    let result = git.try_run_in(path, ["rev-parse", "--show-toplevel"])?;
    if result.success {
        let root = result.stdout.trim();
        if !root.is_empty() {
            return normalize_path(Path::new(root));
        }
    }

    normalize_path(path)
}

fn git_common_dir(git: &Git, path: &Path) -> Result<PathBuf> {
    let output = git.run_in(
        path,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    normalize_path(Path::new(output.stdout.trim()))
}

fn normalize_path(path: &Path) -> Result<PathBuf> {
    Ok(std::fs::canonicalize(path)?)
}

pub(crate) fn repository_id(git_common_dir: &Path) -> String {
    let identity = normalized_identity(git_common_dir);
    let digest = Sha256::digest(identity.as_bytes());
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    format!("sha256:{hex}")
}

fn normalized_identity(path: &Path) -> String {
    let mut value = path.to_string_lossy().into_owned();

    if cfg!(windows) {
        value = value.replace('\\', "/");
    }

    value
}

fn repository_name(main_worktree: Option<&PathBuf>, git_common_dir: &Path) -> String {
    let source = main_worktree
        .map(PathBuf::as_path)
        .unwrap_or(git_common_dir);
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repository");

    name.strip_suffix(".git").unwrap_or(name).to_string()
}

fn config_parent_dir(path: &Path) -> Option<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
}

fn current_config_schema_version() -> u32 {
    CONFIG_SCHEMA_VERSION
}

fn default_worktree_parent_template() -> String {
    "{main_path}.worktrees".to_string()
}

fn default_worktree_name_template() -> String {
    "{repo_name}__{ref_slug}".to_string()
}

fn default_worktree_ref_slug_algorithm() -> String {
    "path_safe_v1".to_string()
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    fn saved_repository(id: &str, name: &str, path: PathBuf) -> SavedRepository {
        SavedRepository {
            id: id.to_string(),
            name: name.to_string(),
            kind: SavedRepositoryKind::WorktreeFamily,
            main_worktree: Some(path.clone()),
            git_common_dir: path.join(".git"),
            saved_from: path,
        }
    }

    fn git(dir: &Path, args: &[&str]) {
        let global_config = dir.join("empty-global-gitconfig");
        let system_config = dir.join("empty-system-gitconfig");
        std::fs::write(&global_config, "").expect("write empty global gitconfig");
        std::fs::write(&system_config, "").expect("write empty system gitconfig");

        let output = Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", &global_config)
            .env("GIT_CONFIG_SYSTEM", &system_config)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn serializes_and_deserializes_config() {
        let mut config = AppConfig::default();
        config.repositories.push(saved_repository(
            "sha256:one",
            "one",
            PathBuf::from("/repo/one"),
        ));

        let json = serde_json::to_string(&config).expect("serialize config");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse config json");
        let loaded: AppConfig = serde_json::from_str(&json).expect("deserialize config");

        assert_eq!(value["schema_version"], 1);
        assert_eq!(
            value["settings"]["worktree"]["parent_template"],
            "{main_path}.worktrees"
        );
        assert_eq!(
            value["settings"]["worktree"]["name_template"],
            "{repo_name}__{ref_slug}"
        );
        assert_eq!(
            value["settings"]["worktree"]["ref_slug_algorithm"],
            "path_safe_v1"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn default_config_uses_schema_v1_and_default_worktree_settings() {
        let config = AppConfig::default();

        assert_eq!(config.schema_version, 1);
        assert_eq!(
            config.settings.worktree.parent_template,
            "{main_path}.worktrees"
        );
        assert_eq!(
            config.settings.worktree.name_template,
            "{repo_name}__{ref_slug}"
        );
        assert_eq!(config.settings.worktree.ref_slug_algorithm, "path_safe_v1");
    }

    #[test]
    fn prevents_duplicate_repositories() {
        let mut config = AppConfig::default();

        assert!(config.add_repository(saved_repository(
            "sha256:one",
            "one",
            PathBuf::from("/repo/one")
        )));
        assert!(!config.add_repository(saved_repository(
            "sha256:one",
            "one-copy",
            PathBuf::from("/repo/one-copy")
        )));
        assert_eq!(config.repositories.len(), 1);
    }

    #[test]
    fn allows_distinct_repository_families() {
        let mut config = AppConfig::default();

        assert!(config.add_repository(saved_repository(
            "sha256:one",
            "one",
            PathBuf::from("/repo/one")
        )));
        assert!(config.add_repository(saved_repository(
            "sha256:two",
            "two",
            PathBuf::from("/repo/two")
        )));
        assert_eq!(config.repositories.len(), 2);
    }

    #[test]
    fn save_writes_loadable_config() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let store = ConfigStore::new(temp_dir.path().join("config.json"));
        let mut config = AppConfig::default();
        config.repositories.push(saved_repository(
            "sha256:one",
            "one",
            PathBuf::from("/repo/one"),
        ));

        store.save(&config).expect("save config");
        let raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(store.path()).expect("read config"))
                .expect("parse saved config");
        let loaded = store.load().expect("load config");

        assert_eq!(raw["schema_version"], 1);
        assert_eq!(
            raw["settings"]["worktree"]["parent_template"],
            "{main_path}.worktrees"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn save_replaces_existing_config() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let store = ConfigStore::new(temp_dir.path().join("config.json"));
        let mut first_config = AppConfig::default();
        first_config.repositories.push(saved_repository(
            "sha256:one",
            "one",
            PathBuf::from("/repo/one"),
        ));
        let mut second_config = AppConfig::default();
        second_config.repositories.push(saved_repository(
            "sha256:two",
            "two",
            PathBuf::from("/repo/two"),
        ));

        store.save(&first_config).expect("save first config");
        store.save(&second_config).expect("replace config");
        let loaded = store.load().expect("load config");

        assert_eq!(loaded, second_config);
    }

    #[test]
    fn load_migrates_legacy_v0_config_without_rewriting_file() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let repo = temp_dir.path().join("repo");
        fs::create_dir(&repo).expect("create repo dir");
        git(&repo, &["init", "-q", "-b", "main"]);
        let path = temp_dir.path().join("config.json");
        let store = ConfigStore::new(path.clone());
        let legacy = serde_json::to_string_pretty(&serde_json::json!({
            "repositories": [
                { "path": repo }
            ]
        }))
        .expect("serialize legacy config");
        fs::write(&path, &legacy).expect("write legacy config");

        let loaded = store.load().expect("load legacy config");
        let raw_after_load = fs::read_to_string(&path).expect("read legacy config");

        assert_eq!(loaded.schema_version, 1);
        assert_eq!(loaded.repositories.len(), 1);
        assert_eq!(
            loaded.repositories[0].kind,
            SavedRepositoryKind::WorktreeFamily
        );
        assert_eq!(
            loaded.repositories[0].main_worktree,
            Some(repo.canonicalize().expect("canonical repo"))
        );
        assert_eq!(raw_after_load, legacy);
    }

    #[test]
    fn load_fills_missing_v1_worktree_setting_fields() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let path = temp_dir.path().join("config.json");
        let store = ConfigStore::new(path.clone());
        fs::write(
            &path,
            r#"{
  "schema_version": 1,
  "settings": {
    "worktree": {
      "parent_template": "{main_path}.custom-worktrees"
    }
  },
  "repositories": []
}"#,
        )
        .expect("write partial v1 config");

        let loaded = store.load().expect("load partial v1 config");

        assert_eq!(
            loaded.settings.worktree.parent_template,
            "{main_path}.custom-worktrees"
        );
        assert_eq!(
            loaded.settings.worktree.name_template,
            "{repo_name}__{ref_slug}"
        );
        assert_eq!(loaded.settings.worktree.ref_slug_algorithm, "path_safe_v1");
    }

    #[test]
    fn load_rejects_unknown_future_schema_version() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let path = temp_dir.path().join("config.json");
        let store = ConfigStore::new(path.clone());
        fs::write(
            &path,
            r#"{
  "schema_version": 999,
  "settings": {},
  "repositories": []
}"#,
        )
        .expect("write future config");

        let err = store.load().expect_err("future schema should fail");

        assert!(err
            .to_string()
            .contains("unsupported config schema version: 999"));
    }

    #[test]
    fn app_home_from_home_points_to_config_json() {
        let home = PathBuf::from("super-git-home");

        let app_home = AppHome::from_home(home.clone(), AppHomeSource::EnvSuperGitHome);

        assert_eq!(app_home.home, home);
        assert_eq!(app_home.source, AppHomeSource::EnvSuperGitHome);
        assert_eq!(
            app_home.config_file,
            PathBuf::from("super-git-home").join("config.json")
        );
    }

    #[test]
    fn repository_id_preserves_path_case() {
        assert_ne!(
            repository_id(Path::new("/repo/Repo/.git")),
            repository_id(Path::new("/repo/repo/.git"))
        );
    }
}
