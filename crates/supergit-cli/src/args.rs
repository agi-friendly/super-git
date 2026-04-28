use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "sg")]
#[command(about = "A small CLI-first foundation for super-git")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Check the local environment.
    Doctor,

    /// Manage registered repositories.
    Repo {
        #[command(subcommand)]
        command: RepoCommands,
    },

    /// Show Git status for a repository path or the current directory.
    Status { path: Option<PathBuf> },

    /// Inspect Git worktrees.
    Wt {
        #[command(subcommand)]
        command: WorktreeCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum RepoCommands {
    /// Add a local Git repository to the config file.
    Add { path: PathBuf },

    /// List registered repositories.
    List,
}

#[derive(Debug, Subcommand)]
pub enum WorktreeCommands {
    /// List worktrees for a repository path or the current directory.
    List { path: Option<PathBuf> },
}
