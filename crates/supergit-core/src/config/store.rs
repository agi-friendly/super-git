use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::git::repository::validate_repository_path;
use crate::model::Repository;
use crate::{Result, SuperGitError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub repositories: Vec<Repository>,
}

impl AppConfig {
    pub fn add_repository_path(&mut self, path: PathBuf) -> bool {
        if self.repositories.iter().any(|repo| repo.path == path) {
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
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(config)?;
        fs::write(&self.path, format!("{content}\n"))?;
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
}
