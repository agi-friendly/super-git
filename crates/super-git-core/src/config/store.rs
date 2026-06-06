use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::git::repository::validate_repository_path;
use crate::model::Repository;
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
    pub repositories: Vec<Repository>,
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
    fn from_legacy_repositories(repositories: Vec<Repository>) -> Self {
        Self {
            repositories,
            ..Self::default()
        }
    }

    pub fn add_repository_path(&mut self, path: PathBuf) -> bool {
        let new_identity = RepositoryIdentity::from_path(&path);

        if self
            .repositories
            .iter()
            .any(|repo| RepositoryIdentity::from_path(&repo.path) == new_identity)
        {
            return false;
        }

        self.repositories.push(Repository::new(path));
        true
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

impl Default for WorktreeSettings {
    fn default() -> Self {
        Self {
            parent_template: default_worktree_parent_template(),
            name_template: default_worktree_name_template(),
            ref_slug_algorithm: default_worktree_ref_slug_algorithm(),
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
pub struct AddRepositoryResult {
    pub path: PathBuf,
    pub added: bool,
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
        if !self.path.exists() {
            return Ok(AppConfig::default());
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

    pub fn add_repository(&self, path: &Path) -> Result<AddRepositoryResult> {
        let normalized = validate_repository_path(path)?;
        let mut config = self.load()?;
        let added = config.add_repository_path(normalized.clone());

        if added {
            self.save(&config)?;
        }

        Ok(AddRepositoryResult {
            path: normalized,
            added,
        })
    }
}

#[derive(Debug, Deserialize)]
struct LegacyAppConfig {
    #[serde(default)]
    repositories: Vec<Repository>,
}

fn parse_config(content: &str) -> Result<AppConfig> {
    let value: serde_json::Value = serde_json::from_str(content)?;

    match value.get("schema_version") {
        None => {
            let legacy: LegacyAppConfig = serde_json::from_value(value)?;
            Ok(AppConfig::from_legacy_repositories(legacy.repositories))
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

            Ok(serde_json::from_value(value)?)
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepositoryIdentity(String);

impl RepositoryIdentity {
    fn from_path(path: &Path) -> Self {
        let mut value = path.to_string_lossy().into_owned();

        if cfg!(windows) {
            value = value.replace('\\', "/");
        }

        if cfg!(any(target_os = "macos", windows)) {
            value = value.to_lowercase();
        }

        Self(value)
    }
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
    use super::*;

    #[test]
    fn serializes_and_deserializes_config() {
        let mut config = AppConfig::default();
        config
            .repositories
            .push(Repository::new(PathBuf::from("/repo/one")));

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

        assert!(config.add_repository_path(PathBuf::from("/repo/one")));
        assert!(!config.add_repository_path(PathBuf::from("/repo/one")));
        assert_eq!(config.repositories.len(), 1);
    }

    #[test]
    fn prevents_case_variant_duplicates_on_case_insensitive_targets() {
        let mut config = AppConfig::default();

        assert!(config.add_repository_path(PathBuf::from("/Repo/One")));
        let second_added = config.add_repository_path(PathBuf::from("/repo/one"));

        if cfg!(any(target_os = "macos", windows)) {
            assert!(!second_added);
            assert_eq!(config.repositories.len(), 1);
        } else {
            assert!(second_added);
            assert_eq!(config.repositories.len(), 2);
        }
    }

    #[test]
    fn save_writes_loadable_config() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let store = ConfigStore::new(temp_dir.path().join("config.json"));
        let mut config = AppConfig::default();
        config
            .repositories
            .push(Repository::new(PathBuf::from("/repo/one")));

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
        first_config
            .repositories
            .push(Repository::new(PathBuf::from("/repo/one")));
        let mut second_config = AppConfig::default();
        second_config
            .repositories
            .push(Repository::new(PathBuf::from("/repo/two")));

        store.save(&first_config).expect("save first config");
        store.save(&second_config).expect("replace config");
        let loaded = store.load().expect("load config");

        assert_eq!(loaded, second_config);
    }

    #[test]
    fn load_migrates_legacy_v0_config_without_rewriting_file() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let path = temp_dir.path().join("config.json");
        let store = ConfigStore::new(path.clone());
        let legacy = r#"{
  "repositories": [
    { "path": "/repo/one" }
  ]
}"#;
        fs::write(&path, legacy).expect("write legacy config");

        let loaded = store.load().expect("load legacy config");
        let raw_after_load = fs::read_to_string(&path).expect("read legacy config");

        assert_eq!(loaded.schema_version, 1);
        assert_eq!(
            loaded.repositories,
            vec![Repository::new(PathBuf::from("/repo/one"))]
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
}
