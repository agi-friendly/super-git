use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::git::repository::validate_repository_path;
use crate::model::Repository;
use crate::{Result, SuperGitError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub repositories: Vec<Repository>,
}

impl AppConfig {
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

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
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
        Ok(serde_json::from_str(&content)?)
    }

    pub fn save(&self, config: &AppConfig) -> Result<()> {
        let parent = config_parent_dir(&self.path);

        if let Some(parent) = parent {
            fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(config)?;
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

pub fn default_config_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "super-git", "super-git")
        .ok_or(SuperGitError::ConfigDirectoryUnavailable)?;

    Ok(dirs.config_dir().join("config.json"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_and_deserializes_config() {
        let config = AppConfig {
            repositories: vec![Repository::new(PathBuf::from("/repo/one"))],
        };

        let json = serde_json::to_string(&config).expect("serialize config");
        let loaded: AppConfig = serde_json::from_str(&json).expect("deserialize config");

        assert_eq!(loaded, config);
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
        let config = AppConfig {
            repositories: vec![Repository::new(PathBuf::from("/repo/one"))],
        };

        store.save(&config).expect("save config");
        let loaded = store.load().expect("load config");

        assert_eq!(loaded, config);
    }

    #[test]
    fn save_replaces_existing_config() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let store = ConfigStore::new(temp_dir.path().join("config.json"));
        let first_config = AppConfig {
            repositories: vec![Repository::new(PathBuf::from("/repo/one"))],
        };
        let second_config = AppConfig {
            repositories: vec![Repository::new(PathBuf::from("/repo/two"))],
        };

        store.save(&first_config).expect("save first config");
        store.save(&second_config).expect("replace config");
        let loaded = store.load().expect("load config");

        assert_eq!(loaded, second_config);
    }
}
