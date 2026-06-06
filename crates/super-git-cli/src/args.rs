use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "super-git")]
#[command(about = "A small CLI-first foundation for super-git")]
#[command(version)]
pub struct Cli {
    /// Render human-readable output instead of the default JSON.
    #[arg(long, global = true)]
    pub human: bool,

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

    /// Inspect super-git global configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },

    /// Show Git status for a repository path or the current directory.
    Status { path: Option<PathBuf> },

    /// Inspect full repository state: HEAD and any in-progress operation.
    Inspect { path: Option<PathBuf> },

    /// Build a read-only plan for a future write action.
    Preview {
        #[command(subcommand)]
        command: PreviewCommands,
    },

    /// Execute a previously previewed plan after re-validation.
    Execute {
        /// Plan file to execute. Use '-' to read from stdin.
        #[arg(long)]
        plan: PathBuf,
    },

    /// Undo a write using a validated undo token.
    Undo {
        /// Undo token file. Use '-' to read from stdin.
        #[arg(long)]
        token: PathBuf,
    },

    /// Inspect Git worktrees.
    Wt {
        #[command(subcommand)]
        command: WorktreeCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum RepoCommands {
    /// Compatibility alias for `repo save <path>`.
    Add { path: PathBuf },

    /// Save a Git worktree family to the repository registry.
    Save { path: Option<PathBuf> },

    /// List saved repository families.
    List,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
    /// Show the resolved super-git app home and config file path.
    Path,

    /// Show the resolved config location and loaded config.
    Show,
}

#[derive(Debug, Subcommand)]
pub enum PreviewCommands {
    /// Preview staging all current unstaged and untracked changes.
    StageChanges,
}

#[derive(Debug, Subcommand)]
pub enum WorktreeCommands {
    /// List worktrees for a repository path or the current directory.
    List { path: Option<PathBuf> },
}
